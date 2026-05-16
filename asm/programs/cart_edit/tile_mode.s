; ============================================================================
; programs/cart_edit/tile_mode.s -- tile-editing chrome and handlers.
;
; Owns: zoom canvas, palette strip, tiled-preview, pixel cursor, flag bits,
;       and all "tile-mode" key handlers. The do_pix_*/do_strip_*/do_page_*
;       handlers double-dispatch to map_mode handlers when ED_MODE != 0.
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"frame.inc"
		.include	"maplayout.inc"
		.include	"tilemap.inc"
		.include	"state.inc"

		.import	sheet_ptr_lo
		.import	sheet_ptr_hi
		.import	hgr_lo
		.import	hgr_hi
		.import	update_status
		.import	paint_map_cursor
		.import	do_map_up
		.import	do_map_dn
		.import	do_map_lt
		.import	do_map_rt
		.import	do_map_paint
		.import	cell_index
		.import	mflag_addr_for

		.export	enter_tile_mode
		.export	hgr_wipe
		.export	paint_strip
		.export	paint_strip_cursor
		.export	paint_canvas
		.export	paint_tiled_preview
		.export	paint_pixel_cursor
		.export	toggle_pixel
		.export	refresh_tile
		.export	tile_ptr
		.export	do_pix_up
		.export	do_pix_dn
		.export	do_pix_lt
		.export	do_pix_rt
		.export	do_toggle
		.export	do_strip_up
		.export	do_strip_dn
		.export	do_strip_lt
		.export	do_strip_rt
		.export	do_page_prev
		.export	do_page_next
		.export	do_flag0
		.export	do_flag1
		.export	do_flag2
		.export	do_flag3
		.export	do_flag4
		.export	do_flag5
		.export	do_flag6
		.export	do_flag7
		.export	do_palette_lo
		.export	do_palette_hi
		.export	do_palette_row_lo
		.export	do_palette_row_hi
		.export	post_select
		.export	post_page

		.segment "CODE"

; ============================================================================
; enter_tile_mode -- paint the tile-editor chrome (canvas, preview,
; palette strip + cursors).
; ============================================================================
		.proc	enter_tile_mode
		JSR	hgr_wipe		; wipe HGR1 (no map writes happen here)
		; Clear the viewport dirty bitmap so the next ENGINE_FRAME_END
		; doesn't run JT_MAP_DRAW_DIRTY and overwrite the palette strip
		; with whatever is in the map at rows 8..9.
		LDA	#0
		LDX	#29
@cd:		STA	TILEMAP_DIRTY,X
		DEX
		BPL	@cd
		JSR	paint_strip
		JSR	paint_strip_cursor
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_pixel_cursor
		JSR	update_status
		RTS
		.endproc

; hgr_wipe -- zero HGR1 ($2000..$3FFF). Inline; the JT_HGR_CLEAR slot is
; unimplemented in the engine right now.
		.proc	hgr_wipe
		LDA	#$00
		STA	ENG_PTR
		LDA	#$20
		STA	ENG_PTR + 1
		LDX	#$20			; 32 pages
		LDY	#0
		LDA	#0
@l:	STA	(ENG_PTR),Y
		INY
		BNE	@l
		INC	ENG_PTR + 1
		DEX
		BNE	@l
		RTS
		.endproc

; ============================================================================
; paint_strip -- splat STRIP_TILES tiles starting at SP into viewport
; cells (col 0..19, rows 8..9) via direct JT_TILE_DRAW. Does NOT touch
; the map; the underlying world map is undisturbed.
; ============================================================================
		.proc	paint_strip
		LDX	#0
@l:
		TXA
		CMP	#STRIP_COLS
		BCC	@row8
		SBC	#STRIP_COLS		; col = I - 20 (carry set from CMP)
		STA	ARG1
		LDA	#9
		BRA	@setrow
@row8:
		STA	ARG1			; col = I
		LDA	#8
@setrow:
		STA	ARG2
		TXA
		CLC
		ADC	ED_SP			; tile = SP + I
		STA	ARG0
		PHX
		JSR	JT_TILE_DRAW
		PLX
		INX
		CPX	#STRIP_TILES
		BNE	@l
		RTS
		.endproc

; ============================================================================
; paint_strip_cursor -- XOR 14x16 box around the slot.
; ============================================================================
		.proc	paint_strip_cursor
		LDA	ED_SS
		CMP	#STRIP_COLS
		BCC	@row8
		SBC	#STRIP_COLS		; col = SS - 20
		STA	ARG0
		LDA	#9
		BRA	@xor
@row8:
		STA	ARG0			; col = SS
		LDA	#8
@xor:
		STA	ARG1
		JSR	JT_TILES_STRIP_CURSOR
		RTS
		.endproc

; ============================================================================
; paint_canvas -- blit current tile T at zoom 7x into upper-left corner.
; ============================================================================
		.proc	paint_canvas
		LDA	ED_T
		STA	ARG0
		JSR	JT_TILE_EDIT_BLIT
		RTS
		.endproc

; ============================================================================
; paint_tiled_preview -- splat ED_T at native pixel size in a
; PREV_W_TILES x PREV_H_TILES grid starting at HGR (PREV_COL_BYTE, PREV_ROW).
; ============================================================================
		.proc	paint_tiled_preview
		; --- ENG_PTR2 = sheet_ptr + ED_T*32
		LDA	ED_T
		STA	ENG_PTR2
		STZ	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1		; *32
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1

		LDA	#0
		STA	ED_TMP			; src_row 0..15
@srow:
		; load 2 src bytes for this row -> ED_TMP2 (lo), ED_TMP3 (hi)
		; Keep bit 7 intact -- it's the per-byte HGR palette select.
		LDA	ED_TMP
		ASL
		TAY
		LDA	(ENG_PTR2),Y
		STA	ED_TMP2
		INY
		LDA	(ENG_PTR2),Y
		STA	ED_TMP3

		LDX	#0			; prev_row 0..PREV_H_TILES-1
@prow:
		TXA
		ASL
		ASL
		ASL
		ASL				; *16
		CLC
		ADC	ED_TMP
		ADC	#PREV_ROW
		PHX
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	#PREV_COL_BYTE
		STA	ENG_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ENG_PTR + 1
		PLX

		LDY	#0
		PHX
		LDX	#PREV_W_TILES
@col:
		LDA	ED_TMP2
		STA	(ENG_PTR),Y
		INY
		LDA	ED_TMP3
		STA	(ENG_PTR),Y
		INY
		DEX
		BNE	@col
		PLX

		INX
		CPX	#PREV_H_TILES
		BNE	@prow

		INC	ED_TMP
		LDA	ED_TMP
		CMP	#16
		BNE	@srow
		RTS
		.endproc

; ============================================================================
; paint_pixel_cursor -- XOR a 14x7 (2-byte-wide) cursor at (CX, CY).
; The pair-edit cursor covers BOTH bytes of the current 2-pixel pair so
; you can see at a glance what 7-px column you're editing.
; ============================================================================
		.proc	paint_pixel_cursor
		LDA	ED_CX
		STA	ARG0
		LDA	ED_CY
		STA	ARG1
		JSR	JT_TILE_EDIT_CURSOR
		LDA	ED_CX
		INC
		STA	ARG0
		LDA	ED_CY
		STA	ARG1
		JSR	JT_TILE_EDIT_CURSOR
		RTS
		.endproc

; sync_pixel_cursor -- if the blink ticker has the cursor in its
; "hidden" XOR phase, draw it back so the caller's erase-XOR actually
; erases. Then reset the blink phase so the next blink_tick treats the
; cursor as freshly visible. Called at the top of every pixel-cursor
; mutator (move, toggle).
		.proc	sync_pixel_cursor
		LDA	ED_BLINK
		BEQ	:+
		JSR	paint_pixel_cursor	; un-hide
:		STZ	ED_TICK
		STZ	ED_BLINK
		RTS
		.endproc

; ============================================================================
; toggle_pixel -- flip the pixel at (CX, CY) in tile T's sheet bytes.
; ============================================================================
		.proc	toggle_pixel
		; --- ENG_PTR2 = sheet_ptr + T*32 + CY*2
		LDA	ED_T
		STA	ENG_PTR2
		STZ	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1		; *32
		LDA	ED_CY
		ASL				; *2
		CLC
		ADC	ENG_PTR2
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	#0
		STA	ENG_PTR2 + 1
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1

		LDA	ED_CX
		CMP	#7
		BCC	@lo
		SEC
		SBC	#7
		STA	ED_TMP			; bit
		LDY	#1
		BRA	@bit
@lo:
		STA	ED_TMP
		LDY	#0
@bit:
		LDA	#$01
		LDX	ED_TMP
		BEQ	@gotmask
@shl:
		ASL
		DEX
		BNE	@shl
@gotmask:
		STA	ED_TMP			; mask
		LDA	(ENG_PTR2),Y
		EOR	ED_TMP
		STA	(ENG_PTR2),Y
		RTS
		.endproc

; ============================================================================
; row_ptr_setup -- ENG_PTR2 := sheet_ptr + ED_T*32 + ED_CY*2  (row base).
; Lo byte at (ENG_PTR2),0; hi byte at (ENG_PTR2),1.
; ============================================================================
		.proc	row_ptr_setup
		LDA	ED_T
		STA	ENG_PTR2
		STZ	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1		; *32
		LDA	ED_CY
		ASL				; *2
		CLC
		ADC	ENG_PTR2
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	#0
		STA	ENG_PTR2 + 1
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1
		RTS
		.endproc

; ============================================================================
; read_bit_col(A=col 0..13) -- A = 0 or 1. Requires ENG_PTR2 = row base.
; Clobbers X, Y.
; ============================================================================
		.proc	read_bit_col
		CMP	#7
		BCC	@lo
		SEC
		SBC	#7
		TAX
		LDY	#1
		BRA	@rd
@lo:
		TAX
		LDY	#0
@rd:
		LDA	(ENG_PTR2),Y
@sh:
		CPX	#0
		BEQ	@done
		LSR
		DEX
		BRA	@sh
@done:
		AND	#$01
		RTS
		.endproc

; ============================================================================
; cycle_pair -- read 2-bit value at columns (CX, CX+1) of ED_T row CY,
; cycle 0 -> 1 -> 2 -> 3 -> 0, and toggle whichever bits changed.
; Pair encoding (left bit << 1) | right bit:
;   00 black, 01 col1, 10 col2, 11 white  (col1/col2 depend on palette)
; CX is always even (cursor steps by 2), so the pair may be entirely in
; the lo byte (CX=0,2,4), entirely in the hi byte (CX=8,10,12), or
; straddle the boundary (CX=6: left bit in lo, right bit in hi).
; toggle_pixel handles the per-bit byte selection internally.
; ============================================================================
		.proc	cycle_pair
		JSR	row_ptr_setup		; ENG_PTR2 = row base
		; --- read left bit at CX into ED_TMP2 ---
		LDA	ED_CX
		JSR	read_bit_col
		ASL				; left << 1
		STA	ED_TMP2			; partial pair
		; --- read right bit at CX+1 ---
		LDA	ED_CX
		CLC
		ADC	#1
		JSR	read_bit_col
		ORA	ED_TMP2
		STA	ED_TMP2			; oldPair (0..3)
		CLC
		ADC	#1
		AND	#$03			; newPair
		EOR	ED_TMP2			; diff bits (bit1=L, bit0=R)
		STA	ED_TMP2			; diff
		; --- toggle right bit (CX+1) if diff bit 0 set ---
		LSR	ED_TMP2			; carry = right diff
		BCC	@noR
		INC	ED_CX
		JSR	toggle_pixel
		DEC	ED_CX
@noR:
		; --- toggle left bit (CX) if diff bit 1 set (now in carry-position) ---
		LSR	ED_TMP2			; carry = left diff
		BCC	@noL
		JSR	toggle_pixel
@noL:
		RTS
		.endproc

; ============================================================================
; tile_ptr -- ENG_PTR2 := sheet_ptr + ED_T*32. Used by clipboard helpers.
; ============================================================================
		.proc	tile_ptr
		LDA	ED_T
		STA	ENG_PTR2
		STZ	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1		; *32
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1
		RTS
		.endproc

; ============================================================================
; refresh_tile -- repaint canvas + preview + strip slot after a tile data
; mutation (paste/clear).
; ============================================================================
		.proc	refresh_tile
		JSR	paint_pixel_cursor	; hide cursor
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_strip_cursor	; erase
		LDA	ED_T
		STA	ARG0
		LDA	ED_SS
		AND	#$07
		STA	ARG1
		LDA	ED_SS
		LSR
		LSR
		LSR
		CLC
		ADC	#8
		STA	ARG2
		JSR	JT_TILE_DRAW		; direct viewport blit
		JSR	paint_strip_cursor	; redraw at new slot
		JSR	paint_pixel_cursor
		RTS
		.endproc

; ----- pixel / map cursor handlers --------------------------------------
; Tiles are 14 wide x 16 tall in storage, but only the top 14 rows of the
; zoom canvas are visible -- the bottom 32 px of the HGR area is taken by
; the palette strip + status text. Cap the editor cursor at 14 rows so it
; can't wander into pixels we have no way to render or address.
		.proc	do_pix_up
		LDA	ED_MODE
		BEQ	:+
		JMP	do_map_up
:		JSR	sync_pixel_cursor
		JSR	paint_pixel_cursor	; erase old
		LDA	ED_CY
		BNE	:+
		LDA	#16
:		DEC
		STA	ED_CY
		JSR	paint_pixel_cursor
		JSR	update_status
		RTS
		.endproc

		.proc	do_pix_dn
		LDA	ED_MODE
		BEQ	:+
		JMP	do_map_dn
:		JSR	sync_pixel_cursor
		JSR	paint_pixel_cursor
		LDA	ED_CY
		INC
		CMP	#16
		BNE	:+
		LDA	#0
:		STA	ED_CY
		JSR	paint_pixel_cursor
		JSR	update_status
		RTS
		.endproc

		.proc	do_pix_lt
		LDA	ED_MODE
		BEQ	:+
		JMP	do_map_lt
:		JSR	sync_pixel_cursor
		JSR	paint_pixel_cursor
		LDA	ED_CX
		BNE	:+
		LDA	#14
:		SEC
		SBC	#2			; pair step
		STA	ED_CX
		JSR	paint_pixel_cursor
		RTS
		.endproc

		.proc	do_pix_rt
		LDA	ED_MODE
		BEQ	:+
		JMP	do_map_rt
:		JSR	sync_pixel_cursor
		JSR	paint_pixel_cursor
		LDA	ED_CX
		CLC
		ADC	#2			; pair step
		CMP	#14
		BNE	:+
		LDA	#0
:		STA	ED_CX
		JSR	paint_pixel_cursor
		RTS
		.endproc

; ----- pixel toggle / map paint -----------------------------------------
		.proc	do_toggle
		LDA	ED_MODE
		BEQ	:+
		JMP	do_map_paint
:		JSR	sync_pixel_cursor
		JSR	paint_pixel_cursor	; hide cursor
		JSR	cycle_pair
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_strip_cursor
		LDA	ED_T
		STA	ARG0
		LDA	ED_SS
		CMP	#STRIP_COLS
		BCC	@row8
		SBC	#STRIP_COLS
		STA	ARG1
		LDA	#9
		BRA	@setrow
@row8:
		STA	ARG1
		LDA	#8
@setrow:
		STA	ARG2
		JSR	JT_TILE_DRAW
		JSR	paint_strip_cursor
		JSR	paint_pixel_cursor
		RTS
		.endproc

; ----- strip selector ---------------------------------------------------
		.proc	do_strip_up
		JSR	maybe_strip_cursor
		LDA	ED_SS
		SEC
		SBC	#STRIP_COLS
		BPL	:+
		CLC
		ADC	#STRIP_TILES
:		STA	ED_SS
		JSR	post_select
		RTS
		.endproc

		.proc	do_strip_dn
		JSR	maybe_strip_cursor
		LDA	ED_SS
		CLC
		ADC	#STRIP_COLS
		CMP	#STRIP_TILES
		BCC	:+
		SEC
		SBC	#STRIP_TILES
:		STA	ED_SS
		JSR	post_select
		RTS
		.endproc

		.proc	do_strip_lt
		JSR	maybe_strip_cursor
		LDA	ED_SS
		BNE	:+
		LDA	#STRIP_TILES
:		DEC
		STA	ED_SS
		JSR	post_select
		RTS
		.endproc

		.proc	do_strip_rt
		JSR	maybe_strip_cursor
		LDA	ED_SS
		INC
		CMP	#STRIP_TILES
		BCC	:+
		LDA	#0
:		STA	ED_SS
		JSR	post_select
		RTS
		.endproc

; maybe_strip_cursor -- erase strip cursor only in tile mode.
		.proc	maybe_strip_cursor
		LDA	ED_MODE
		BNE	:+
		JMP	paint_strip_cursor
:		RTS
		.endproc

; ----- strip page -------------------------------------------------------
; Pages step by STRIP_TILES (40). Valid SP values: 0, 40, 80, 120, 160, 200.
; Tiles 240..255 are reachable only by editing ED_T directly (no page lands
; on them); good enough for now since most projects won't fill 240 tiles.
		.proc	do_page_prev
		LDA	ED_SP
		SEC
		SBC	#STRIP_TILES
		BCS	:+
		LDA	#200
:		STA	ED_SP
		JSR	post_page
		RTS
		.endproc

		.proc	do_page_next
		LDA	ED_SP
		CLC
		ADC	#STRIP_TILES
		CMP	#240
		BCC	:+
		LDA	#0
:		STA	ED_SP
		JSR	post_page
		RTS
		.endproc

; ----- flag toggles -----------------------------------------------------
; Toggle bit N of CART_FLAGS[ED_T], then refresh status row.
		.proc	do_flag0
		LDA	#$01
		BRA	flag_xor
		.endproc
		.proc	do_flag1
		LDA	#$02
		BRA	flag_xor
		.endproc
		.proc	do_flag2
		LDA	#$04
		BRA	flag_xor
		.endproc
		.proc	do_flag3
		LDA	#$08
		BRA	flag_xor
		.endproc
		.proc	do_flag4
		LDA	#$10
		BRA	flag_xor
		.endproc
		.proc	do_flag5
		LDA	#$20
		BRA	flag_xor
		.endproc
		.proc	do_flag6
		LDA	#$40
		BRA	flag_xor
		.endproc
		.proc	do_flag7
		LDA	#$80
		; fall through
		.endproc
		.proc	flag_xor
		; A = mask byte; mode-dispatch.
		LDX	ED_MODE
		BEQ	@sprite
		; --- map mode: XOR mask into mflags[ED_LAYER] at world (ED_MX,ED_MY)
		PHA				; save mask
		LDA	ED_MY
		LDX	ED_MX
		JSR	cell_index		; idx16 in (idx_lo, idx_hi)
		LDA	ED_LAYER
		JSR	mflag_addr_for		; ENG_PTR -> mflag byte
		PLA				; mask
		LDY	#0
		EOR	(ENG_PTR),Y
		STA	(ENG_PTR),Y
		JSR	update_status
		RTS
@sprite:
		; --- tile mode: XOR into SPRITE_FLAGS[ED_T] (per tile-id sprite flag)
		LDX	ED_T
		EOR	SPRITE_FLAGS,X
		STA	SPRITE_FLAGS,X
		JSR	update_status
		RTS
		.endproc

; ----- after a strip slot change ------------------------------------------
		.proc	post_select
		LDA	ED_SP
		CLC
		ADC	ED_SS
		STA	ED_T
		LDA	ED_MODE
		BEQ	:+
		JSR	paint_map_cursor	; redraw brush preview with new T
		JSR	update_status
		RTS
:		JSR	paint_strip_cursor	; redraw at new slot
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_pixel_cursor
		JSR	update_status
		RTS
		.endproc

; ----- after a page change ------------------------------------------------
		.proc	post_page
		LDA	ED_SP
		CLC
		ADC	ED_SS
		STA	ED_T
		LDA	ED_MODE
		BEQ	:+
		JSR	paint_map_cursor
		JSR	update_status
		RTS
:		JSR	paint_strip_cursor	; erase old
		JSR	paint_strip
		JSR	paint_strip_cursor	; redraw at slot
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_pixel_cursor
		JSR	update_status
		RTS
		.endproc

; ============================================================================
; do_palette_lo / do_palette_hi -- toggle the HGR palette bit (bit 7) of
; the left or right byte of the current tile, for ALL 16 rows. Each tile
; byte is 7 pixels wide; bit 7 = 0 -> green/violet palette, bit 7 = 1 ->
; orange/blue palette. Tile-mode only.
; ============================================================================
		.proc	do_palette_lo
		LDA	ED_MODE
		BEQ	:+
		RTS
:		LDA	#0
		STA	ED_TMP			; byte index in row: 0 = lo
		JMP	palette_toggle_common
		.endproc

		.proc	do_palette_hi
		LDA	ED_MODE
		BEQ	:+
		RTS
:		LDA	#1
		STA	ED_TMP			; byte index in row: 1 = hi
		JMP	palette_toggle_common
		.endproc

; palette_toggle_common -- ED_TMP = byte-in-row (0=lo, 1=hi). Toggles bit 7
; of that byte for all 16 rows of ED_T. Refreshes preview + canvas + status.
		.proc	palette_toggle_common
		JSR	tile_ptr		; ENG_PTR2 = tile base
		LDA	ED_TMP
		STA	ED_TMP2			; running Y = byte index in row
		LDX	#16			; 16 rows
@l:
		LDY	ED_TMP2
		LDA	(ENG_PTR2),Y
		EOR	#$80
		STA	(ENG_PTR2),Y
		; advance Y by 2 (next row, same byte-in-row)
		LDA	ED_TMP2
		CLC
		ADC	#2
		STA	ED_TMP2
		DEX
		BNE	@l
		JSR	paint_pixel_cursor	; hide cursor
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_pixel_cursor	; restore
		JSR	update_status
		RTS
		.endproc

; ============================================================================
; do_palette_row_lo / do_palette_row_hi -- toggle bit 7 of the lo / hi byte
; of just the CURRENT pixel-cursor row (ED_CY) of ED_T. Per-row palette flip.
; Tile-mode only.
; ============================================================================
		.proc	do_palette_row_lo
		LDA	ED_MODE
		BEQ	:+
		RTS
:		LDA	#0
		STA	ED_TMP			; byte index in row: 0 = lo
		JMP	palette_row_common
		.endproc

		.proc	do_palette_row_hi
		LDA	ED_MODE
		BEQ	:+
		RTS
:		LDA	#1
		STA	ED_TMP			; byte index in row: 1 = hi
		JMP	palette_row_common
		.endproc

; palette_row_common -- ED_TMP = byte-in-row (0=lo, 1=hi). Toggles bit 7
; of that byte for ED_CY only. Refreshes the affected canvas row + preview.
		.proc	palette_row_common
		JSR	tile_ptr		; ENG_PTR2 = tile base
		; Y = ED_CY * 2 + ED_TMP
		LDA	ED_CY
		ASL
		CLC
		ADC	ED_TMP
		TAY
		LDA	(ENG_PTR2),Y
		EOR	#$80
		STA	(ENG_PTR2),Y
		JSR	paint_pixel_cursor	; hide cursor
		JSR	paint_canvas
		JSR	paint_tiled_preview
		JSR	paint_pixel_cursor	; restore
		JSR	update_status
		RTS
		.endproc
