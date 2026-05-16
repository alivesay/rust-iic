; ============================================================================
; lib/edit.s -- IDE editor helpers (zoom-render a tile + XOR cursor block).
;
; Used by the BASIC sprite/tile editor. Renders a single tile at zoom 7
; (98x112 px) in the upper-left HGR region; lets BASIC POKE pixel bits in
; the cart sheet then re-blit on each toggle.
;
; Why zoom 7? 14 src px * 7 = 98 dst px = 14 dst bytes (HGR is 7 px/byte),
; so each src bit maps to exactly one full dst byte ($00 or $7F). No bit
; spreading across byte boundaries -> simple, fast, mono.
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"tilemap.inc"
		.include	"edit.inc"

		.import	hgr_lo
		.import	hgr_hi
		.import	sheet_ptr_lo
		.import	sheet_ptr_hi

		.export	edit_init
		.export	tile_edit_blit
		.export	tile_edit_cursor
		.export	tiles_strip_cursor

; ZP scratch:
;   ENG_PTR  ($06/$07)  -- HGR dest row pointer
;   ENG_PTR2 ($08/$09)  -- src tile pointer
;   ENG_TMP  ($0A)      -- src row counter (0..15)
;   ENG_TMP2 ($0B)      -- src col counter (0..13)
;   ENG_TMP3 ($0C)      -- zoom row sub-counter (0..6)
;   ENG_TMP4 ($0D)      -- absolute HGR pixel row

; ----- BSS ------------------------------------------------------------------
		.segment "BSS"
edit_rowbuf:	.res	14		; one expanded src row, 14 bytes
edit_lo_on:	.res	1		; "on" pixel pattern for lo byte (palette)
edit_hi_on:	.res	1		; "on" pixel pattern for hi byte

		.segment "CODE"

; ----------------------------------------------------------------------------
; edit_init -- wire ABI slots.
; ----------------------------------------------------------------------------
		.proc	edit_init
		JT_SET_SLOT JT_TILE_EDIT_BLIT,     jt_tile_edit_blit
		JT_SET_SLOT JT_TILE_EDIT_CURSOR,   jt_tile_edit_cursor
		JT_SET_SLOT JT_TILES_STRIP_CURSOR, jt_tiles_strip_cursor
		RTS
		.endproc

		.proc	jt_tile_edit_blit
		LDA	ARG0
		JSR	tile_edit_blit
		CLC
		RTS
		.endproc

		.proc	jt_tile_edit_cursor
		LDA	ARG0
		LDX	ARG1
		JSR	tile_edit_cursor
		CLC
		RTS
		.endproc

; ----------------------------------------------------------------------------
; tile_edit_blit(A=tile_id) -- render zoomed tile to canvas region.
; ----------------------------------------------------------------------------
		.proc	tile_edit_blit
		; --- ENG_PTR2 = sheet_ptr + tile*32
		STA	ENG_PTR2
		LDA	#0
		STA	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1

		LDA	#0
		STA	ENG_TMP			; src row
@srow:
		; --- expand src row N (2 bytes) into edit_rowbuf (14 bytes)
		LDA	ENG_TMP
		ASL
		TAY
		LDA	(ENG_PTR2),Y		; lo byte (cols 0..6)
		PHA
		INY
		LDA	(ENG_PTR2),Y		; hi byte (cols 7..13)
		STA	ENG_TMP6
		PLA
		; A = lo byte; ENG_TMP6 = hi byte
		; --- pick fill pattern for "on" pixels per byte's palette bit
		;     palette 0 (bit7=0) -> $7F  (mono in canvas; HGR draws white
		;                                  for solid 7-bit run)
		;     palette 1 (bit7=1) -> $FF  (high bit set; visible byte-level
		;                                  shift in canvas as a half-pixel
		;                                  offset = palette indicator)
		; The small native-size preview is the source of truth for color.
		PHA				; save lo byte
		AND	#$80
		BEQ	@lo_p0
		LDA	#$FF
		STA	edit_lo_on
		BRA	@lo_done
@lo_p0:
		LDA	#$7F
		STA	edit_lo_on
@lo_done:
		LDA	ENG_TMP6
		AND	#$80
		BEQ	@hi_p0
		LDA	#$FF
		STA	edit_hi_on
		BRA	@hi_done
@hi_p0:
		LDA	#$7F
		STA	edit_hi_on
@hi_done:
		PLA				; restore lo byte
		; expand bits 0..6 of A
		LDX	#0
@expand_lo:
		LSR
		BCC	@lo_off
		PHA
		LDA	edit_lo_on
		STA	edit_rowbuf,X
		BRA	@lo_next2
@lo_off:
		PHA
		LDA	#$00
		STA	edit_rowbuf,X
@lo_next2:
		PLA
		INX
		CPX	#7
		BNE	@expand_lo
		; expand bits 0..6 of hi byte
		LDA	ENG_TMP6
@expand_hi:
		LSR
		BCC	@hi_off
		PHA
		LDA	edit_hi_on
		STA	edit_rowbuf,X
		BRA	@hi_next2
@hi_off:
		PHA
		LDA	#$00
		STA	edit_rowbuf,X
@hi_next2:
		PLA
		INX
		CPX	#14
		BNE	@expand_hi

		; --- splat that row buf to 7 dst HGR rows
		LDA	#0
		STA	ENG_TMP3		; zoom sub-row 0..6
@zrow:
		; abs_row = EDIT_DST_ROW + ENG_TMP*7 + ENG_TMP3
		LDA	ENG_TMP
		ASL
		ASL
		ASL				; *8
		SEC
		SBC	ENG_TMP			; *7
		CLC
		ADC	ENG_TMP3
		ADC	#EDIT_DST_ROW
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	#EDIT_DST_COL_BYTE
		STA	ENG_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ENG_PTR + 1
		; copy 14 bytes from edit_rowbuf to (ENG_PTR),0..13
		LDY	#13
@cpy:
		LDA	edit_rowbuf,Y
		STA	(ENG_PTR),Y
		DEY
		BPL	@cpy

		INC	ENG_TMP3
		LDA	ENG_TMP3
		CMP	#EDIT_ZOOM
		BNE	@zrow

		INC	ENG_TMP
		LDA	ENG_TMP
		CMP	#EDIT_SRC_H
		BEQ	@srow_done
		JMP	@srow
@srow_done:

		RTS
		.endproc

; ----------------------------------------------------------------------------
; tile_edit_cursor(A=col 0..13, X=row 0..15) -- XOR a 7x7 inverse block.
; Call once to draw the cursor, again with the same args to erase it.
; ----------------------------------------------------------------------------
		.proc	tile_edit_cursor
		STA	ENG_TMP2		; col
		STX	ENG_TMP			; row
		; abs_col_byte = EDIT_DST_COL_BYTE + col   (1 src px = 1 dst byte)
		; abs_row_base = EDIT_DST_ROW + row*7
		LDA	#0
		STA	ENG_TMP3		; zoom sub-row 0..6
@zrow:
		LDA	ENG_TMP
		ASL
		ASL
		ASL				; *8
		SEC
		SBC	ENG_TMP			; *7
		CLC
		ADC	ENG_TMP3
		ADC	#EDIT_DST_ROW
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	ENG_TMP2
		ADC	#EDIT_DST_COL_BYTE
		STA	ENG_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ENG_PTR + 1
		LDY	#0
		LDA	(ENG_PTR),Y
		EOR	#$7F
		STA	(ENG_PTR),Y
		INC	ENG_TMP3
		LDA	ENG_TMP3
		CMP	#EDIT_ZOOM
		BNE	@zrow
		RTS
		.endproc

; ----------------------------------------------------------------------------
; jt_tiles_strip_cursor -- ABI wrapper.
; ----------------------------------------------------------------------------
		.proc	jt_tiles_strip_cursor
		LDA	ARG0
		LDX	ARG1
		JSR	tiles_strip_cursor
		CLC
		RTS
		.endproc

; ----------------------------------------------------------------------------
; tiles_strip_cursor(A=col, X=row) -- XOR a 14x16 (= 2 byte * 16 row)
; rectangle at map cell (col,row). Call again with the same args to undo.
; ----------------------------------------------------------------------------
		.proc	tiles_strip_cursor
		STA	ENG_TMP2		; col
		ASL	ENG_TMP2		; col_byte = col * 2
		STX	ENG_TMP			; row
		; pixel_row_base = row * 16
		TXA
		ASL
		ASL
		ASL
		ASL
		STA	ENG_TMP3
		LDA	#0
		STA	ENG_TMP4		; sub-row 0..15
@zrow:
		LDA	ENG_TMP3
		CLC
		ADC	ENG_TMP4
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	ENG_TMP2
		STA	ENG_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ENG_PTR + 1
		; XOR both bytes (left + right halves of the 14 px row)
		LDY	#0
		LDA	(ENG_PTR),Y
		EOR	#$7F
		STA	(ENG_PTR),Y
		INY
		LDA	(ENG_PTR),Y
		EOR	#$7F
		STA	(ENG_PTR),Y
		INC	ENG_TMP4
		LDA	ENG_TMP4
		CMP	#16
		BNE	@zrow
		RTS
		.endproc
