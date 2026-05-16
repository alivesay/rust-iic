; ============================================================================
; programs/cart_edit/status.s -- mixed-mode text status rows.
;
; Row 20: "TILE nnn   PAGE nnn   SLOT nn F:bbbbbbbb"
; Row 21: mode badge ("TILE MODE ..." / "MAP MODE X:nn Y:nn ...")
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"maplayout.inc"
		.include	"tilemap.inc"
		.include	"state.inc"

		.import	cell_index
		.import	mflag_addr_for
		.import	ED_RH
		.import	ED_RW
		.import	sheet_ptr_lo
		.import	sheet_ptr_hi

		.export	clear_text_rows
		.export	update_status
		.export	fmt_dec3
		.export	fmt_dec2
		.export	status_buf

; ============================================================================
; clear_text_rows -- write space ($A0 = ' '|$80) to all 4 mixed-mode rows.
; Row addresses on the 40-col text page: 20=$0650, 21=$06D0, 22=$0750, 23=$07D0.
; ============================================================================
		.proc	clear_text_rows
		LDA	#$A0
		LDX	#39
@r20:	STA	$0650,X
		DEX
		BPL	@r20
		LDX	#39
@r21:	STA	$06D0,X
		DEX
		BPL	@r21
		LDX	#39
@r22:	STA	$0750,X
		DEX
		BPL	@r22
		LDX	#39
@r23:	STA	$07D0,X
		DEX
		BPL	@r23
		RTS
		.endproc

; ============================================================================
; update_status -- format and paint the two status rows.
; ============================================================================
		.proc	update_status
		; --- copy template into status_buf
		LDX	#STATUS_LEN-1
@cp:	LDA	status_tpl,X
		STA	status_buf,X
		DEX
		BPL	@cp

		; --- T (3 digits at offset 5)
		LDA	ED_T
		LDY	#T_DIGIT_OFF
		JSR	fmt_dec3
		; --- SP (3 digits at offset 16)
		LDA	ED_SP
		LDY	#P_DIGIT_OFF
		JSR	fmt_dec3
		; --- SS (2 digits at offset 27)
		LDA	ED_SS
		LDY	#S_DIGIT_OFF
		JSR	fmt_dec2

		; --- 8 flag bits at offset 32..39 (bit 0 leftmost)
		LDX	ED_T
		LDA	SPRITE_FLAGS,X
		STA	ED_TMP
		LDX	#0
@fb:	LDA	#'0'
		LSR	ED_TMP
		BCC	:+
		LDA	#'1'
:		STA	status_buf+F_DIGIT_OFF,X
		INX
		CPX	#8
		BNE	@fb

		; --- copy buffer to text row 20 with bit 7 set (normal video)
		LDX	#STATUS_LEN-1
@out:	LDA	status_buf,X
		ORA	#$80
		STA	$0650,X
		DEX
		BPL	@out

		; --- row 21: mode badge + (in map mode) cursor coords
		LDA	ED_MODE
		BEQ	@use_tile
		; resize-prompt template overrides MAP MODE template
		LDA	ED_RESIZE
		BEQ	@map_tpl
		LDX	#<mode_resize_tpl
		LDY	#>mode_resize_tpl
		BRA	@have_tpl
@map_tpl:
		LDX	#<mode_map_tpl
		LDY	#>mode_map_tpl
		BRA	@have_tpl
@use_tile:
		LDX	#<mode_tile_tpl
		LDY	#>mode_tile_tpl
@have_tpl:
		STX	ENG_PTR
		STY	ENG_PTR + 1
		LDY	#STATUS_LEN-1
@cp2:	LDA	(ENG_PTR),Y
		STA	status_buf,Y
		DEY
		BPL	@cp2

		LDA	ED_MODE
		BEQ	@tile_fields
		LDA	ED_RESIZE
		BEQ	@map_fields
		; --- resize prompt: fill ED_RW @ col 7, ED_RH @ col 12
		LDA	ED_RW
		LDY	#7
		JSR	fmt_dec2
		LDA	ED_RH
		LDY	#12
		JSR	fmt_dec2
		BRA	@row21_out
@map_fields:
		; Layer digit at col 5
		LDA	ED_LAYER
		CLC
		ADC	#'0'
		STA	status_buf+5
		; X digits at col 9..10
		LDA	ED_MX
		LDY	#9
		JSR	fmt_dec2
		; Y digits at col 14..15
		LDA	ED_MY
		LDY	#14
		JSR	fmt_dec2
		; --- 8 map-flag bits at col 19..26 (current cell, current layer)
		LDA	ED_MY
		LDX	ED_MX
		JSR	cell_index
		LDA	ED_LAYER
		JSR	mflag_addr_for
		LDY	#0
		LDA	(ENG_PTR),Y
		STA	ED_TMP
		LDX	#0
@mfb:	LDA	#'0'
		LSR	ED_TMP
		BCC	:+
		LDA	#'1'
:		STA	status_buf+19,X
		INX
		CPX	#8
		BNE	@mfb
		BRA	@row21_out
@tile_fields:
		; --- TILE MODE: fill palette indicator P:LR for the row under
		;     the pixel cursor (ED_CY). L = bit 7 of lo byte, R = hi.
		JSR	tile_ptr_status
		LDA	ED_CY
		ASL
		TAY				; Y = CY*2  (lo byte of row)
		LDA	(ENG_PTR),Y
		ASL				; bit 7 -> carry
		LDA	#'0'
		BCC	:+
		LDA	#'1'
:		STA	status_buf+7
		LDA	ED_CY
		ASL
		CLC
		ADC	#1
		TAY				; Y = CY*2+1 (hi byte of row)
		LDA	(ENG_PTR),Y
		ASL
		LDA	#'0'
		BCC	:+
		LDA	#'1'
:		STA	status_buf+8
@row21_out:
		LDX	#STATUS_LEN-1
@out2:	LDA	status_buf,X
		ORA	#$80
		STA	$06D0,X
		DEX
		BPL	@out2
		RTS
		.endproc

; ----- fmt_dec3: A = value (0..255), Y = buffer offset --------------------
		.proc	fmt_dec3
		LDX	#$FF			; hundreds
@h:	INX
		SEC
		SBC	#100
		BCS	@h
		ADC	#100
		PHA
		TXA
		ORA	#'0'
		STA	status_buf,Y
		PLA
		INY

		LDX	#$FF			; tens
@t:	INX
		SEC
		SBC	#10
		BCS	@t
		ADC	#10
		PHA
		TXA
		ORA	#'0'
		STA	status_buf,Y
		PLA
		INY

		ORA	#'0'			; ones
		STA	status_buf,Y

		; --- replace leading '0' with ' '
		DEY
		DEY
		LDA	status_buf,Y
		CMP	#'0'
		BNE	@done
		LDA	#' '
		STA	status_buf,Y
		INY
		LDA	status_buf,Y
		CMP	#'0'
		BNE	@done
		LDA	#' '
		STA	status_buf,Y
@done:
		RTS
		.endproc

; ----- fmt_dec2: A = value (0..99), Y = buffer offset ---------------------
		.proc	fmt_dec2
		LDX	#$FF
@t:	INX
		SEC
		SBC	#10
		BCS	@t
		ADC	#10
		PHA
		TXA
		ORA	#'0'
		STA	status_buf,Y
		PLA
		INY
		ORA	#'0'
		STA	status_buf,Y
		; replace leading zero
		DEY
		LDA	status_buf,Y
		CMP	#'0'
		BNE	@done
		LDA	#' '
		STA	status_buf,Y
@done:
		RTS
		.endproc

; ----- tile_ptr_status: ENG_PTR := sheet_ptr + ED_T*32 -------------------
		.proc	tile_ptr_status
		LDA	ED_T
		STA	ENG_PTR
		STZ	ENG_PTR + 1
		ASL	ENG_PTR
		ROL	ENG_PTR + 1
		ASL	ENG_PTR
		ROL	ENG_PTR + 1
		ASL	ENG_PTR
		ROL	ENG_PTR + 1
		ASL	ENG_PTR
		ROL	ENG_PTR + 1
		ASL	ENG_PTR
		ROL	ENG_PTR + 1
		CLC
		LDA	ENG_PTR
		ADC	sheet_ptr_lo
		STA	ENG_PTR
		LDA	ENG_PTR + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR + 1
		RTS
		.endproc

; ----- data ---------------------------------------------------------------
status_tpl:
		.byte	"TILE 000   PAGE 000   SLOT 00 F:00000000"
mode_tile_tpl:
		.byte	"TILE P:00 ,.=tile ;'=row        TAB MAP"
mode_map_tpl:
		.byte	"MAP L0 X:00 Y:00 F:00000000 N=LYR R=size"
mode_resize_tpl:
		.byte	"RESIZE W:00 H:00  IJKL  RET=ok  ESC=cnl "

		.segment "BSS"
status_buf:	.res	40
