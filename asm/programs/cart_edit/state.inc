; ============================================================================
; programs/cart_edit/state.inc -- shared ZP state, mode constants, and
; layout offsets used across all cart_edit modules.
;
; Included by: cart_edit.s, tile_mode.s, map_mode.s, status.s,
;              clipboard.s, save.s
; ============================================================================

	.ifndef _CART_EDIT_STATE_INC_
_CART_EDIT_STATE_INC_	= 1

; ----- editor state in user ZP ($10-$1F) -----------------------------------
ED_T		= $10	; current tile id
ED_CX		= $11	; pixel cursor X (0..13)
ED_CY		= $12	; pixel cursor Y (0..15)
ED_SP		= $13	; strip page top tile id (0, 40, 80, 120, 160, 200)
ED_SS		= $14	; strip slot 0..39
ED_TMP		= $15
ED_TMP2		= $16
ED_TMP3		= $17
ED_MODE		= $18	; 0 = tile mode, 1 = map mode
ED_MX		= $19	; map cursor col (0..19)
ED_MY		= $1A	; map cursor row (0..11)
ED_TICK		= $1B	; frame counter (drives cursor blink)
ED_BLINK	= $1C	; last-seen blink phase (0 or $10)
ED_LAYER	= $1D	; map mode active layer (0..2)
ED_RESIZE	= $1E	; 0 = normal, 1 = resize prompt active

MODE_TILE	= 0
MODE_MAP	= 1

NUM_LAYERS	= 3

; Pending resize values live in BSS (see cart_edit.s) since they only
; matter while ED_RESIZE != 0:
;   .global ED_RW    pending W
;   .global ED_RH    pending H

; ----- tiled-preview geometry ---------------------------------------------
; Painted at native pixel size to the right of the zoom canvas (which
; occupies HGR col bytes 2..15, rows 0..111). 5 tiles wide x 7 tall = 70 px
; wide (10 bytes) x 112 rows. Sits in mixed-mode HGR area (rows 0..127).
PREV_COL_BYTE	= 18
PREV_ROW	= 0
PREV_W_TILES	= 5
PREV_H_TILES	= 7

; Strip palette occupies map cells (cols 0..19, rows 8..9) -- full HGR
; width below the canvas/preview. Their original tilemap contents are
; stashed in map_save before paint_strip writes the palette over them,
; so we can restore the user's actual map when entering map mode.
STRIP_COLS	= 20			; tiles per strip row
STRIP_TILES	= STRIP_COLS * 2	; 40 visible palette slots per page

; ----- status row layout (used by status.s; defined here so other
; modules can reference offsets if needed in the future) -----------------
STATUS_LEN	= 40
T_DIGIT_OFF	= 5
P_DIGIT_OFF	= 16
S_DIGIT_OFF	= 27
F_DIGIT_OFF	= 32

	.endif	; _CART_EDIT_STATE_INC_
