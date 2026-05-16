; ============================================================================
; lib/jumptable.s -- install the $0300 engine ABI jump table at runtime.
;
; Phase 0: every slot points at `jt_unimpl` (carry-set, A=0). Real procs
; replace the JMP target as each lib lands. Address layout is locked in
; lib/jumptable.inc -- never renumber a published slot.
;
; Cap: $03BF. Page-3 above that holds DOS / ProDOS / Monitor vectors
; ($03D0 BASIC.SYSTEM cold, $03D3 warm, $03F2 reset, $03FE/FF IRQ).
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"

		.export	jumptable_install
		.export	jt_unimpl

JT_TEMPLATE_SIZE = jt_template_end - jt_template

		.segment "CODE"

; ----------------------------------------------------------------------------
; jumptable_install -- copy the jump-table template to $0300.
;
; Call once during engine_init, BEFORE any code JSRs through the table.
; Idempotent (safe to call again).
;
; Clobbers A, X. Preserves Y.
; ----------------------------------------------------------------------------
		.proc	jumptable_install
		LDX	#JT_TEMPLATE_SIZE - 1
@l:
		LDA	jt_template,X
		STA	JT_BASE,X
		DEX
		BPL	@l
		RTS
		.endproc

; ----------------------------------------------------------------------------
; jt_unimpl -- default body for every slot until a real proc replaces it.
;
; Returns:
;   A = 0
;   carry SET   (= error per ABI)
; ----------------------------------------------------------------------------
		.proc	jt_unimpl
		LDA	#0
		SEC
		RTS
		.endproc

; ----------------------------------------------------------------------------
; jt_template -- 22 slots * 3 bytes each = 66 bytes. All point at jt_unimpl
; for now. Real entries get patched in by individual lib init routines (or
; by re-emitting the template once the .export targets exist).
;
; KEEP IN SYNC WITH lib/jumptable.inc slot order.
; ----------------------------------------------------------------------------
jt_template:
		JMP	jt_unimpl	; $0300 tile_draw
		JMP	jt_unimpl	; $0303 tile_set_map
		JMP	jt_unimpl	; $0306 map_draw_all
		JMP	jt_unimpl	; $0309 map_draw_dirty
		JMP	jt_unimpl	; $030C map_scroll
		JMP	jt_unimpl	; $030F sprite_draw
		JMP	jt_unimpl	; $0312 sprite_clear
		JMP	jt_unimpl	; $0315 sfx_play
		JMP	jt_unimpl	; $0318 music_start
		JMP	jt_unimpl	; $031B music_stop
		JMP	jt_unimpl	; $031E music_tick
		JMP	jt_unimpl	; $0321 hgr_clear
		JMP	jt_unimpl	; $0324 hgr_pixel
		JMP	jt_unimpl	; $0327 hgr_line
		JMP	jt_unimpl	; $032A text_draw_char
		JMP	jt_unimpl	; $032D text_draw_str
		JMP	jt_unimpl	; $0330 rng_next
		JMP	jt_unimpl	; $0333 rng_range
		JMP	jt_unimpl	; $0336 wait_vbl
		JMP	jt_unimpl	; $0339 mode_swap
		JMP	jt_unimpl	; $033C win_open
		JMP	jt_unimpl	; $033F win_close
		JMP	jt_unimpl	; $0342 map_fill_rect
		JMP	jt_unimpl	; $0345 map_border
		JMP	jt_unimpl	; $0348 tile_move
		JMP	jt_unimpl	; $034B input_dxdy
		JMP	jt_unimpl	; $034E reserved (was cart_load)
		JMP	jt_unimpl	; $0351 reserved (was cart_splash)
		JMP	jt_unimpl	; $0354 tile_at
		JMP	jt_unimpl	; $0357 tile_edit_blit
		JMP	jt_unimpl	; $035A tile_edit_cursor
		JMP	jt_unimpl	; $035D tiles_strip_cursor
		JMP	jt_unimpl	; $0360 tile_set_map_l
		JMP	jt_unimpl	; $0363 tile_at_l
		JMP	jt_unimpl	; $0366 map_set_cam
		JMP	jt_unimpl	; $0369 map_resize
jt_template_end:
