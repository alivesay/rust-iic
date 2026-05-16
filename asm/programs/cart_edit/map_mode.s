; ============================================================================
; programs/cart_edit/map_mode.s -- variable-size map editor with viewport.
;
; ED_MX/ED_MY are WORLD coordinates clamped to [0, map_w-1]/[0, map_h-1].
; Viewport (cam_x, cam_y) tracks the top-left visible world cell. Cursor
; movement past the visible edge scrolls the camera by 1 cell.
;
; Three painter's-order layers (L0 back, L2 front; tile 0 transparent on
; L1/L2). The brush preview at (ED_MX, ED_MY) is XOR-overlaid so it
; remains visible even when ED_T matches the underlying composite.
;
; Cell access goes through engine helpers:
;   JT_TILE_AT_L / JT_TILE_SET_MAP_L take WORLD coords + layer.
;   cell_index + cell_addr_for / mflag_addr_for are linked directly from
;   the engine for direct ENG_PTR access (used for status flags + paint).
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"maplayout.inc"
		.include	"tilemap.inc"
		.include	"state.inc"

		.import	update_status

		.export	enter_map_mode
		.export	paint_map_cursor
		.export	unpaint_map_cursor
		.export	do_map_up
		.export	do_map_dn
		.export	do_map_lt
		.export	do_map_rt
		.export	do_map_paint
		.export	do_layer_cycle
		.export	cursor_view_col
		.export	cursor_view_row
		.export	clamp_cursor_to_map
		.export	scroll_into_view

		.segment "CODE"

; ============================================================================
; enter_map_mode -- redraw whole viewport, drop cursor.
; (Tile mode no longer writes to the map, so there is nothing to restore.)
; ============================================================================
		.proc	enter_map_mode
		JSR	clamp_cursor_to_map
		JSR	scroll_into_view	; ensure cursor visible
		JSR	JT_MAP_DRAW_ALL
		JSR	paint_map_cursor
		JSR	update_status
		RTS
		.endproc

; ============================================================================
; cursor_view_col / cursor_view_row -- A := ED_MX - cam_x / ED_MY - cam_y.
; Caller must have ensured the cursor is within the viewport already.
; ============================================================================
		.proc	cursor_view_col
		LDA	ED_MX
		SEC
		SBC	cam_x
		RTS
		.endproc

		.proc	cursor_view_row
		LDA	ED_MY
		SEC
		SBC	cam_y
		RTS
		.endproc

; ============================================================================
; paint_map_cursor -- ED_T as brush preview at viewport (vc, vr) =
; (ED_MX-cam_x, ED_MY-cam_y), then XOR cursor box on top.
; ============================================================================
		.proc	paint_map_cursor
		LDA	ED_T
		STA	ARG0
		JSR	cursor_view_col
		STA	ARG1
		JSR	cursor_view_row
		STA	ARG2
		JSR	JT_TILE_DRAW
		JSR	cursor_view_col
		STA	ARG0
		JSR	cursor_view_row
		STA	ARG1
		JSR	JT_TILES_STRIP_CURSOR
		RTS
		.endproc

; ============================================================================
; unpaint_map_cursor -- recompose all 3 layers at (ED_MX, ED_MY) so brush
; preview vanishes. Reads tiles via JT_TILE_AT_L (world coords).
; ============================================================================
		.proc	unpaint_map_cursor
		JSR	cursor_view_col
		STA	ED_TMP			; vc
		JSR	cursor_view_row
		STA	ED_TMP2			; vr
		; --- layer 0 always
		LDA	ED_MX
		STA	ARG0
		LDA	ED_MY
		STA	ARG1
		LDA	#0
		STA	ARG2
		JSR	JT_TILE_AT_L
		LDA	ARG0
		STA	ED_TMP3			; tile
		LDA	ED_TMP
		STA	ARG1
		LDA	ED_TMP2
		STA	ARG2
		LDA	ED_TMP3
		STA	ARG0
		JSR	JT_TILE_DRAW
		; --- layer 1 if non-zero
		LDA	ED_MX
		STA	ARG0
		LDA	ED_MY
		STA	ARG1
		LDA	#1
		STA	ARG2
		JSR	JT_TILE_AT_L
		LDA	ARG0
		BEQ	@l2
		STA	ED_TMP3
		LDA	ED_TMP
		STA	ARG1
		LDA	ED_TMP2
		STA	ARG2
		LDA	ED_TMP3
		STA	ARG0
		JSR	JT_TILE_DRAW
@l2:		LDA	ED_MX
		STA	ARG0
		LDA	ED_MY
		STA	ARG1
		LDA	#2
		STA	ARG2
		JSR	JT_TILE_AT_L
		LDA	ARG0
		BEQ	@done
		STA	ED_TMP3
		LDA	ED_TMP
		STA	ARG1
		LDA	ED_TMP2
		STA	ARG2
		LDA	ED_TMP3
		STA	ARG0
		JSR	JT_TILE_DRAW
@done:		RTS
		.endproc

; ============================================================================
; clamp_cursor_to_map -- clip ED_MX/ED_MY into [0, map_w-1]/[0, map_h-1].
; Useful after a resize that shrinks the map.
; ============================================================================
		.proc	clamp_cursor_to_map
		LDA	map_w
		BEQ	@y			; W=0 shouldn't happen; skip clamp
		DEC
		CMP	ED_MX
		BCS	@y
		STA	ED_MX
@y:
		LDA	map_h
		BEQ	@done
		DEC
		CMP	ED_MY
		BCS	@done
		STA	ED_MY
@done:
		RTS
		.endproc

; ============================================================================
; scroll_into_view -- if cursor is outside the [cam_x..cam_x+VIEW_COLS) x
; [cam_y..cam_y+VIEW_ROWS) viewport, recenter the camera so it fits, then
; call JT_MAP_SET_CAM. Returns Z=1 if cam unchanged, Z=0 otherwise.
; ============================================================================
		.proc	scroll_into_view
		LDA	cam_x
		STA	ED_TMP			; old cx
		LDA	cam_y
		STA	ED_TMP2			; old cy
		; cx clamps so ED_MX in [cx, cx+VIEW_COLS)
		LDA	ED_MX
		CMP	cam_x
		BCS	@xhi
		STA	cam_x			; cursor left of view
		BRA	@xdone
@xhi:
		SEC
		SBC	#VIEW_COLS-1		; ED_MX - (VIEW_COLS-1)
		BCC	@xdone			; underflow -> already in view
		CMP	cam_x
		BCC	@xdone
		STA	cam_x
@xdone:
		; clamp cam_x so cam_x + VIEW_COLS <= map_w
		LDA	map_w
		CMP	#VIEW_COLS
		BCC	@xzero			; map smaller than view
		SEC
		SBC	#VIEW_COLS		; max cx = W - VIEW_COLS
		CMP	cam_x
		BCS	@xclamped
		STA	cam_x
		BRA	@xclamped
@xzero:
		STZ	cam_x
@xclamped:
		LDA	ED_MY
		CMP	cam_y
		BCS	@yhi
		STA	cam_y
		BRA	@ydone
@yhi:
		SEC
		SBC	#VIEW_ROWS-1
		BCC	@ydone
		CMP	cam_y
		BCC	@ydone
		STA	cam_y
@ydone:
		LDA	map_h
		CMP	#VIEW_ROWS
		BCC	@yzero
		SEC
		SBC	#VIEW_ROWS
		CMP	cam_y
		BCS	@yclamped
		STA	cam_y
		BRA	@yclamped
@yzero:
		STZ	cam_y
@yclamped:
		; if cam changed, push to engine so it marks all-dirty
		LDA	cam_x
		CMP	ED_TMP
		BNE	@changed
		LDA	cam_y
		CMP	ED_TMP2
		BNE	@changed
		LDA	#1			; Z=0 -> unchanged? we want Z=1
		EOR	#1
		RTS
@changed:
		LDA	cam_x
		STA	ARG0
		LDA	cam_y
		STA	ARG1
		JSR	JT_MAP_SET_CAM
		LDA	#1			; non-zero => changed
		RTS
		.endproc

; ----- cursor movement (clamp to map; scroll cam if needed) ---------------
; do_map_* sequence:
;   1. JSR unpaint_map_cursor    (erase brush at current cell)
;   2. update ED_MX/ED_MY (clamped, no wrap)
;   3. JSR scroll_into_view      (scroll cam if cursor left viewport)
;      - if scrolled: JT_MAP_DRAW_DIRTY happens via map_set_cam path,
;        but actually map_set_cam just marks all dirty; we still need
;        the next ENGINE_FRAME_END to repaint. Force a draw here so
;        the cursor sits on the new view.
;   4. JSR paint_map_cursor

		.proc	do_map_up
		JSR	unpaint_map_cursor
		LDA	ED_MY
		BEQ	@show
		DEC	ED_MY
@show:		JSR	scroll_into_view
		BEQ	@nodraw
		JSR	JT_MAP_DRAW_ALL
@nodraw:	JSR	paint_map_cursor
		JSR	update_status
		RTS
		.endproc

		.proc	do_map_dn
		JSR	unpaint_map_cursor
		LDA	ED_MY
		INC
		CMP	map_h
		BCC	@ok
		LDA	ED_MY			; clamp; don't wrap
@ok:		STA	ED_MY
		JSR	scroll_into_view
		BEQ	@nodraw
		JSR	JT_MAP_DRAW_ALL
@nodraw:	JSR	paint_map_cursor
		JSR	update_status
		RTS
		.endproc

		.proc	do_map_lt
		JSR	unpaint_map_cursor
		LDA	ED_MX
		BEQ	@show
		DEC	ED_MX
@show:		JSR	scroll_into_view
		BEQ	@nodraw
		JSR	JT_MAP_DRAW_ALL
@nodraw:	JSR	paint_map_cursor
		JSR	update_status
		RTS
		.endproc

		.proc	do_map_rt
		JSR	unpaint_map_cursor
		LDA	ED_MX
		INC
		CMP	map_w
		BCC	@ok
		LDA	ED_MX
@ok:		STA	ED_MX
		JSR	scroll_into_view
		BEQ	@nodraw
		JSR	JT_MAP_DRAW_ALL
@nodraw:	JSR	paint_map_cursor
		JSR	update_status
		RTS
		.endproc

; do_map_paint -- toggle ED_T into map[ED_LAYER][ED_MY*W + ED_MX]: writing
; the same tile twice clears it (writes 0). Then redraw cell + brush.
		.proc	do_map_paint
		; read current tile at this cell+layer
		LDA	ED_MX
		STA	ARG0
		LDA	ED_MY
		STA	ARG1
		LDA	ED_LAYER
		STA	ARG2
		JSR	JT_TILE_AT_L
		LDA	ARG0
		CMP	ED_T
		BNE	@put
		LDA	#0			; toggle erase
		BRA	@write
@put:		LDA	ED_T
@write:		STA	ARG0
		LDA	ED_MX
		STA	ARG1
		LDA	ED_MY
		STA	ARG2
		LDA	ED_LAYER
		STA	ARG3
		JSR	JT_TILE_SET_MAP_L
		JSR	JT_MAP_DRAW_DIRTY
		JSR	paint_map_cursor	; brush preview lost in redraw
		JSR	update_status
		RTS
		.endproc

; do_layer_cycle -- N key. ED_LAYER 0->1->2->0 in MAP MODE only.
		.proc	do_layer_cycle
		LDA	ED_MODE
		BEQ	@done
		LDA	ED_LAYER
		INC
		CMP	#NUM_LAYERS
		BCC	:+
		LDA	#0
:		STA	ED_LAYER
		JSR	update_status
@done:
		RTS
		.endproc
