; ============================================================================
; programs/lib_test/lib_test.s -- Phase 1 smoke test for the engine lib.
;
; What it proves:
;   * engine_init brings the system up (jumptable + per-lib inits + HGR).
;   * tilemap_init wired the four $0300 slots; we call them as BASIC would.
;   * input_publish writes the $0260 page each frame.
;   * map_draw_dirty repaints only what changed.
;   * The whole thing is reachable as a `BRUN` from DOS 3.3.
;
; Behavior: builds a 20x12 map with wall border + floor interior + a
; player tile, then loops reading the input page; IJKL moves the player
; one cell at a time using JT_TILE_SET_MAP + map_draw_dirty.
;
; This is an asm test harness. The same lib will be driven from BASIC
; in Phase 1.5 once the boot-disk authoring path lands.
; ============================================================================

		.setcpu	"65C02"
		.include	"softswitches.inc"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"input.inc"
		.include	"tilemap.inc"
		.include	"frame.inc"

		.segment "GAME"

; ----- player state ---------------------------------------------------------
		.proc	start
		SEI
		CLD
		LDX	#$FF
		TXS
		CLI
		JSR	engine_init
		JSR	build_initial_map
		JSR	map_draw_all_via_abi
		LDA	#10
		STA	plr_x
		LDA	#6
		STA	plr_y
		JSR	draw_player
@frame:
		JSR	frame_begin_via_abi
		JSR	handle_input
		JSR	frame_end_via_abi
		JMP	@frame
		.endproc

; ----------------------------------------------------------------------------
; build_initial_map -- write walls (tile 1) on border, floors (tile 2) inside.
; Uses tile_set_map (= JT_TILE_SET_MAP) so dirty bits get marked too.
; ----------------------------------------------------------------------------
		.proc	build_initial_map
		LDA	#0
		STA	ARG2		; row = 0
@row:		LDA	#0
		STA	ARG1		; col = 0
@col:
		; pick tile: wall (1) on edges, floor (2) inside
		LDA	#1		; default = wall
		LDX	ARG2
		BEQ	@is_wall	; row 0
		CPX	#TILEMAP_ROWS - 1
		BEQ	@is_wall	; last row
		LDX	ARG1
		BEQ	@is_wall	; col 0
		CPX	#TILEMAP_COLS - 1
		BEQ	@is_wall	; last col
		LDA	#2		; floor
@is_wall:
		STA	ARG0
		JSR	JT_TILE_SET_MAP
		INC	ARG1
		LDA	ARG1
		CMP	#TILEMAP_COLS
		BNE	@col
		INC	ARG2
		LDA	ARG2
		CMP	#TILEMAP_ROWS
		BNE	@row
		RTS
		.endproc

; ----------------------------------------------------------------------------
; draw_player -- write player tile (3) at (plr_x, plr_y) via JT_TILE_SET_MAP.
; ----------------------------------------------------------------------------
		.proc	draw_player
		LDA	#3
		STA	ARG0
		LDA	plr_x
		STA	ARG1
		LDA	plr_y
		STA	ARG2
		JSR	JT_TILE_SET_MAP
		RTS
		.endproc

; ----------------------------------------------------------------------------
; clear_player -- write floor (2) at the OLD player position.
; ----------------------------------------------------------------------------
		.proc	clear_player
		LDA	#2
		STA	ARG0
		LDA	plr_x
		STA	ARG1
		LDA	plr_y
		STA	ARG2
		JSR	JT_TILE_SET_MAP
		RTS
		.endproc

; ----------------------------------------------------------------------------
; map_draw_all_via_abi -- call $0306 to redraw everything once after build.
; ----------------------------------------------------------------------------
		.proc	map_draw_all_via_abi
		JSR	JT_MAP_DRAW_ALL
		RTS
		.endproc

; ----------------------------------------------------------------------------
; frame_begin_via_abi / frame_end_via_abi -- BASIC would do CALL 947 / 950.
; We do the equivalent JSR through the page-3 lifecycle slots.
; ----------------------------------------------------------------------------
		.proc	frame_begin_via_abi
		JSR	ENGINE_FRAME_BEGIN
		RTS
		.endproc

		.proc	frame_end_via_abi
		JSR	ENGINE_FRAME_END
		RTS
		.endproc

; ----------------------------------------------------------------------------
; handle_input -- check $0260 page; on direction press, move player.
; Uses tile_set_map for both clear-old and draw-new so map_draw_dirty
; only repaints those two cells.
; ----------------------------------------------------------------------------
		.proc	handle_input
		LDA	INPUT_BTN_UP
		BEQ	@try_dn
		LDA	plr_y
		CMP	#1		; can't go above row 1 (wall at row 0)
		BCC	@done
		BEQ	@done
		JSR	clear_player
		DEC	plr_y
		JSR	draw_player
		RTS
@try_dn:
		LDA	INPUT_BTN_DN
		BEQ	@try_lt
		LDA	plr_y
		CMP	#TILEMAP_ROWS - 2
		BCS	@done
		JSR	clear_player
		INC	plr_y
		JSR	draw_player
		RTS
@try_lt:
		LDA	INPUT_BTN_LT
		BEQ	@try_rt
		LDA	plr_x
		CMP	#1
		BCC	@done
		BEQ	@done
		JSR	clear_player
		DEC	plr_x
		JSR	draw_player
		RTS
@try_rt:
		LDA	INPUT_BTN_RT
		BEQ	@done
		LDA	plr_x
		CMP	#TILEMAP_COLS - 2
		BCS	@done
		JSR	clear_player
		INC	plr_x
		JSR	draw_player
@done:		RTS
		.endproc

; ----- player state ---------------------------------------------------------
plr_x:		.byte	10
plr_y:		.byte	6

; ----- HGR row tables now live in lib/hgr.s ---------------------------------
; (no per-program emit needed)
