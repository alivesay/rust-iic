; ============================================================================
; programs/cart_edit/clipboard.s -- 32-byte tile clipboard.
; C = copy, P = paste, X = clear.
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"state.inc"

		.import	tile_ptr
		.import	refresh_tile

		.export	do_copy
		.export	do_paste
		.export	do_clear

		.segment "CODE"

; do_copy -- copy 32 bytes of current tile -> clipboard.
		.proc	do_copy
		JSR	tile_ptr
		LDY	#31
@l:	LDA	(ENG_PTR2),Y
		STA	clipboard,Y
		DEY
		BPL	@l
		RTS
		.endproc

; do_paste -- copy 32 bytes clipboard -> current tile, refresh views.
		.proc	do_paste
		JSR	tile_ptr
		LDY	#31
@l:	LDA	clipboard,Y
		STA	(ENG_PTR2),Y
		DEY
		BPL	@l
		JMP	refresh_tile
		.endproc

; do_clear -- zero 32 bytes of current tile, refresh views.
		.proc	do_clear
		JSR	tile_ptr
		LDA	#0
		LDY	#31
@l:	STA	(ENG_PTR2),Y
		DEY
		BPL	@l
		JMP	refresh_tile
		.endproc

		.segment "BSS"
clipboard:	.res	32
