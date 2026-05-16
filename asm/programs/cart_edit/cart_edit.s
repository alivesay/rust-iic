; ============================================================================
; programs/cart_edit/cart_edit.s -- standalone cart editor (sprites + map).
;
; Self-contained asm program. Links engine_init + all libs + the sibling
; cart_edit_*.s modules (status, tile_mode, map_mode, clipboard, save).
;
; Boot:
;   1. HELLO BLOADs CART     -> $4000
;   2. HELLO BLOADs CARTEDIT -> $8000
;   3. HELLO CALL 32768
;   -> engine_init, mixed mode, cart_load, main loop
;
; Modes (TAB toggles):
;   TILE MODE: edit individual sprites at zoom-7. Status row 20:
;     "TILE nnn   PAGE nnn   SLOT  nn F:bbbbbbbb"
;     IJKL = pixel cursor; SPACE = toggle pixel; WASD = palette slot;
;     [ ] = palette page; 1..8 = toggle flag bit; C/P/X = copy/paste/clear;
;     V = save (BSAVE CART back to disk via STARTUP loop).
;   MAP MODE: paint cells of the 20x12 tilemap with the selected tile.
;     IJKL = map cursor; RETURN/SPACE = paint cell.
;
; This file owns: program entry, main loop, blink ticker, key dispatcher,
;                 TAB mode-toggle. Everything else lives in a sibling .s.
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"frame.inc"
		.include	"input.inc"
		.include	"softswitches.inc"
		.include	"macros.inc"
		.include	"tilemap.inc"
		.include	"state.inc"

		.import	engine_init

		; from status.s
		.import	clear_text_rows
		.import	update_status

		; from tile_mode.s
		.import	enter_tile_mode
		.import	paint_pixel_cursor
		.import	do_pix_up, do_pix_dn, do_pix_lt, do_pix_rt
		.import	do_toggle
		.import	do_strip_up, do_strip_dn, do_strip_lt, do_strip_rt
		.import	do_page_prev, do_page_next
		.import	do_flag0, do_flag1, do_flag2, do_flag3
		.import	do_flag4, do_flag5, do_flag6, do_flag7
		.import	do_palette_lo, do_palette_hi
		.import	do_palette_row_lo, do_palette_row_hi

		; from map_mode.s
		.import	enter_map_mode
		.import	paint_map_cursor
		.import	unpaint_map_cursor
		.import	do_map_paint
		.import	do_layer_cycle

		; from clipboard.s
		.import	do_copy, do_paste, do_clear

		; from save.s
		.import	do_save

		.export	cart_edit_main
		.export	ED_RW
		.export	ED_RH

		.segment "GAME"
		JMP	cart_edit_main

		.segment "BSS"
ED_RW:		.res	1			; pending W during resize prompt
ED_RH:		.res	1			; pending H during resize prompt

		.segment "CODE"

; ============================================================================
; cart_edit_main -- entry. CALL 32768 from BASIC HELLO.
; ============================================================================
		.proc	cart_edit_main
		JSR	engine_init		; HGR1, JT install, lib init
		LDA	$C053			; MIXED on (HGR top + 4 text lines)

		; TILES, SFLAGS, MAP files were BLOAD'd by STARTUP into
		; $4000, $6000, $6100 respectively. Engine sheet_ptr already
		; points at $4000 from tilemap_init. Mark every cell dirty so
		; the first map_draw_dirty paints from the loaded data.
		LDA	#$FF
		LDX	#30
@dirty:		STA	TILEMAP_DIRTY - 1,X
		DEX
		BNE	@dirty

		; --- editor state defaults
		LDA	#1
		STA	ED_T
		STA	ED_SS
		LDA	#0
		STA	ED_CX
		STA	ED_CY
		STA	ED_SP
		STA	ED_MODE			; start in tile mode
		LDA	#10
		STA	ED_MX
		LDA	#5
		STA	ED_MY
		STZ	ED_TICK
		STZ	ED_BLINK
		STZ	ED_LAYER
		STZ	ED_RESIZE

		JSR	clear_text_rows
		JSR	enter_tile_mode

@loop:
		JSR	ENGINE_FRAME_BEGIN	; input_publish
		LDA	INPUT_LASTKEY
		BEQ	@noinput
		PHA
		STZ	INPUT_LASTKEY
		PLA
		JSR	handle_key
@noinput:
		JSR	ENGINE_FRAME_END	; map_draw_dirty
		JSR	blink_tick
		WAIT_VBL
		BRA	@loop
		.endproc

; ============================================================================
; blink_tick -- bump ED_TICK; on bit-4 transitions toggle the active
; cursor. Tile mode: XOR the pixel cursor. Map mode: paint/unpaint the
; brush preview. Phase 0 = visible, $10 = hidden.
; ============================================================================
		.proc	blink_tick
		INC	ED_TICK
		LDA	ED_TICK
		AND	#$10
		CMP	ED_BLINK
		BEQ	@done
		STA	ED_BLINK
		LDA	ED_MODE
		BNE	@map
		JSR	paint_pixel_cursor	; XOR toggle
		RTS
@map:
		LDA	ED_BLINK
		BNE	@hide
		JSR	paint_map_cursor	; show brush
		RTS
@hide:
		JSR	unpaint_map_cursor
@done:
		RTS
		.endproc

; ============================================================================
; do_toggle_mode -- TAB swaps tile/map mode.
; ============================================================================
		.proc	do_toggle_mode
		LDA	ED_MODE
		EOR	#1
		STA	ED_MODE
		BNE	@to_map
		JSR	enter_tile_mode
		RTS
@to_map:
		JSR	enter_map_mode
		RTS
		.endproc

; ============================================================================
; handle_key(A=ascii) -- one-shot key dispatch via small table.
; Each entry: 1 byte ascii, 2 bytes handler addr. Table terminated by $00.
; ============================================================================
		.proc	handle_key
		STA	ED_TMP			; save key
		LDA	ED_RESIZE
		BEQ	@normal
		LDA	ED_TMP
		JMP	resize_key
@normal:
		LDX	#0
@scan:
		LDA	key_table,X
		BEQ	@done			; sentinel -> unknown key
		CMP	ED_TMP
		BEQ	@hit
		INX
		INX
		INX
		BRA	@scan
@hit:
		INX
		LDA	key_table,X
		STA	ENG_PTR
		INX
		LDA	key_table,X
		STA	ENG_PTR + 1
		JMP	(ENG_PTR)		; tail-call handler (handler RTSs)
@done:
		RTS
		.endproc

key_table:
		.byte	'I'
		.word	do_pix_up
		.byte	'i'
		.word	do_pix_up
		.byte	$0B
		.word	do_pix_up
		.byte	'K'
		.word	do_pix_dn
		.byte	'k'
		.word	do_pix_dn
		.byte	$0A
		.word	do_pix_dn
		.byte	'J'
		.word	do_pix_lt
		.byte	'j'
		.word	do_pix_lt
		.byte	$08
		.word	do_pix_lt
		.byte	'L'
		.word	do_pix_rt
		.byte	'l'
		.word	do_pix_rt
		.byte	$15
		.word	do_pix_rt
		.byte	' '
		.word	do_toggle
		.byte	'W'
		.word	do_strip_up
		.byte	'w'
		.word	do_strip_up
		.byte	'S'
		.word	do_strip_dn
		.byte	's'
		.word	do_strip_dn
		.byte	'A'
		.word	do_strip_lt
		.byte	'a'
		.word	do_strip_lt
		.byte	'D'
		.word	do_strip_rt
		.byte	'd'
		.word	do_strip_rt
		.byte	'['
		.word	do_page_prev
		.byte	']'
		.word	do_page_next
		.byte	'N'
		.word	do_layer_cycle
		.byte	'n'
		.word	do_layer_cycle
		.byte	'1'
		.word	do_flag0
		.byte	'2'
		.word	do_flag1
		.byte	'3'
		.word	do_flag2
		.byte	'4'
		.word	do_flag3
		.byte	'5'
		.word	do_flag4
		.byte	'6'
		.word	do_flag5
		.byte	'7'
		.word	do_flag6
		.byte	'8'
		.word	do_flag7
		.byte	'V'
		.word	do_save
		.byte	'v'
		.word	do_save
		.byte	'C'
		.word	do_copy
		.byte	'c'
		.word	do_copy
		.byte	'P'
		.word	do_paste
		.byte	'p'
		.word	do_paste
		.byte	'X'
		.word	do_clear
		.byte	'x'
		.word	do_clear
		.byte	','
		.word	do_palette_lo
		.byte	'.'
		.word	do_palette_hi
		.byte	';'
		.word	do_palette_row_lo
		.byte	$27			; apostrophe
		.word	do_palette_row_hi
		.byte	$09			; TAB
		.word	do_toggle_mode
		.byte	$0D			; RETURN -- map paint
		.word	do_map_paint
		.byte	'R'
		.word	do_resize_enter
		.byte	'r'
		.word	do_resize_enter
		.byte	0			; sentinel

; ============================================================================
; Resize prompt -- 'R' enters; in resize mode IJKL adjust pending W/H,
; RETURN commits via JT_MAP_RESIZE (zeros all map data!), ESC cancels.
; Bounds: W in [1, MAP_MAX_W], H in [1, MAP_MAX_H], W*H <= MAP_MAX_CELLS.
; ============================================================================
		.import	clamp_cursor_to_map
		.import	scroll_into_view

		.proc	do_resize_enter
		LDA	ED_MODE
		BNE	:+
		RTS				; only valid in MAP MODE
:		LDA	#1
		STA	ED_RESIZE
		LDA	map_w
		STA	ED_RW
		LDA	map_h
		STA	ED_RH
		JSR	update_status
		RTS
		.endproc

; resize_key -- one-shot key dispatch while ED_RESIZE != 0.
; J/L: width -/+; I/K: height -/+; RET commit; ESC cancel.
		.proc	resize_key
		CMP	#$1B			; ESC
		BEQ	@cancel
		CMP	#$0D			; RETURN
		BEQ	@commit
		CMP	#'I'
		BEQ	@hdec
		CMP	#'i'
		BEQ	@hdec
		CMP	#'K'
		BEQ	@hinc
		CMP	#'k'
		BEQ	@hinc
		CMP	#'J'
		BEQ	@wdec
		CMP	#'j'
		BEQ	@wdec
		CMP	#'L'
		BEQ	@winc
		CMP	#'l'
		BEQ	@winc
		RTS				; ignore others while resizing
@wdec:
		LDA	ED_RW
		CMP	#2
		BCC	@upd			; already 1
		DEC	ED_RW
		BRA	@upd
@winc:
		LDA	ED_RW
		CMP	#MAP_MAX_W
		BCS	@upd
		INC	ED_RW
		BRA	@upd
@hdec:
		LDA	ED_RH
		CMP	#2
		BCC	@upd
		DEC	ED_RH
		BRA	@upd
@hinc:
		LDA	ED_RH
		CMP	#MAP_MAX_H
		BCS	@upd
		INC	ED_RH
@upd:
		JSR	update_status
		RTS
@cancel:
		STZ	ED_RESIZE
		JSR	update_status
		RTS
@commit:
		LDA	ED_RW
		STA	ARG0
		LDA	ED_RH
		STA	ARG1
		JSR	JT_MAP_RESIZE
		BCS	@bad			; out of range -> stay in prompt
		STZ	ED_RESIZE
		; cursor / cam may now point past the new bounds
		JSR	clamp_cursor_to_map
		JSR	scroll_into_view
		JSR	JT_MAP_DRAW_ALL
		JSR	paint_map_cursor
		JSR	update_status
		RTS
@bad:
		JSR	update_status
		RTS
		.endproc
