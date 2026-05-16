; ============================================================================
; lib/edit.inc -- IDE editor extern decls.
;
; Helpers for the in-engine sprite/tile editor. All wired into the $0300 ABI
; by edit_init.
; ============================================================================

	.ifndef _LIB_EDIT_INC_
_LIB_EDIT_INC_	= 1

; Editor canvas geometry: tile rendered zoom-7 starting at HGR
; col_byte=2 (px 14), row=0. 14 src px * 7 = 98 dst px wide; 16 src rows
; * 7 = 112 dst rows tall.
EDIT_DST_COL_BYTE	= 2
EDIT_DST_ROW		= 0
EDIT_ZOOM		= 7
EDIT_SRC_W		= 14
EDIT_SRC_H		= 16

	.global	edit_init
	.global	tile_edit_blit
	.global	tile_edit_cursor
	.global	tiles_strip_cursor

	.endif	; _LIB_EDIT_INC_
