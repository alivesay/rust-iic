; ============================================================================
; lib/tilemap.inc -- variable-size tilemap with viewport.
;
; Tile size: 14 px wide x 16 px tall = 2 HGR bytes wide x 16 rows = 32 B/tile.
; Viewport: VIEW_COLS x VIEW_ROWS = 20 x 12 cells (covers 280x192 HGR1).
; Map size: variable, runtime W x H stored in the .MAP header at $6100.
;
; Three layers, painter's-order composite (L0 back, L2 front). Tile id 0
; in layers 1..2 is treated as transparent: that layer skips the cell.
;
; ABI (see jumptable.inc for slot addresses):
;   JT_TILE_DRAW       ARG0=tile, ARG1=col(0..19), ARG2=row(0..11)
;                      Direct HGR blit at viewport cell. Does not touch map.
;   JT_TILE_SET_MAP    ARG0=tile, ARG1=wcol, ARG2=wrow
;                      Layer-0 alias for JT_TILE_SET_MAP_L. WORLD coords.
;   JT_TILE_SET_MAP_L  ARG0=tile, ARG1=wcol, ARG2=wrow, ARG3=layer
;                      Writes map[layer][wrow*MAP_W + wcol]. Marks viewport
;                      cell dirty if (wcol,wrow) is in view.
;   JT_TILE_AT         ARG0=wcol, ARG1=wrow                  -> ARG0 := tile
;   JT_TILE_AT_L       ARG0=wcol, ARG1=wrow, ARG2=layer      -> ARG0 := tile
;   JT_MAP_DRAW_ALL    Mark all viewport cells dirty, repaint.
;   JT_MAP_DRAW_DIRTY  Repaint viewport cells whose dirty bit is set.
;   JT_MAP_SET_CAM     ARG0=cam_x, ARG1=cam_y. Mark all dirty + redraw.
;   JT_MAP_RESIZE      ARG0=W, ARG1=H. Validates W*H<=MAP_MAX_CELLS, zeros
;                      data, writes header. Carry set on bad size.
; ============================================================================

	.ifndef _LIB_TILEMAP_INC_
_LIB_TILEMAP_INC_	= 1

	.include "maplayout.inc"

; --- viewport / dirty bitmap ---------------------------------------------
TILEMAP_COLS	= VIEW_COLS		; legacy alias
TILEMAP_ROWS	= VIEW_ROWS		; legacy alias
TILEMAP_CELLS	= VIEW_CELLS		; 240
TILEMAP_DIRTY	= $0E00			; 30 bytes
TILEMAP_LAYERS	= MAP_LAYERS

TILE_BYTES	= 32			; 2 wide * 16 tall

; --- runtime engine state (BSS in tilemap.s) -----------------------------
	.global	map_w
	.global	map_h
	.global	cam_x
	.global	cam_y
	.global	map_l0_lo, map_l0_hi
	.global	map_l1_lo, map_l1_hi
	.global	map_l2_lo, map_l2_hi
	.global	map_mf0_lo, map_mf0_hi
	.global	map_mf1_lo, map_mf1_hi
	.global	map_mf2_lo, map_mf2_hi

	.global	tilemap_init
	.global	map_init			; validate header / init defaults
	.global	map_recompute_ptrs
	.global	map_set_cam			; A=cx, X=cy
	.global	map_resize			; A=W, X=H; carry set on error
	.global	tile_draw
	.global	tile_set_map
	.global	map_draw_all
	.global	map_draw_dirty
	.global	tile_set_sheet			; X=lo, Y=hi
	.global	sheet_ptr_lo
	.global	sheet_ptr_hi

	; helpers exposed for the editor (run on shared library link):
	.global	cell_index		; A=wrow, X=wcol -> idx in idx_lo/idx_hi
	.global	cell_addr_for		; A=layer, idx pre-loaded -> ENG_PTR
	.global	mflag_addr_for		; A=layer, idx pre-loaded -> ENG_PTR
	.global	idx_lo
	.global	idx_hi

	.endif	; _LIB_TILEMAP_INC_
