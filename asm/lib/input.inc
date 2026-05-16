; ============================================================================
; lib/input.inc -- input layer extern decls.
;
; Input page layout (published every frame_begin):
;   $0260  ANYKEY    1 if a key was pressed this frame, else 0
;   $0261  LASTKEY   ASCII of last key (high bit stripped); 0 if none
;   $0262  BTN_UP    $80 if Up was pressed this frame, else 0
;   $0263  BTN_DN    $80 if Down was pressed
;   $0264  BTN_LT    $80 if Left was pressed
;   $0265  BTN_RT    $80 if Right was pressed
;   $0266  BTN0      $80 if open-apple held (sampled live)
;   $0267  BTN1      $80 if solid-apple held (sampled live)
;
; UP/DN/LT/RT are debounced one-shot per frame. From BASIC:
;   PEEK(610) = BTN_UP, PEEK(611) = BTN_DN,
;   PEEK(612) = BTN_LT, PEEK(613) = BTN_RT.
; ============================================================================

	.ifndef _LIB_INPUT_INC_
_LIB_INPUT_INC_	= 1

INPUT_PAGE	= $0260
INPUT_ANYKEY	= INPUT_PAGE + 0
INPUT_LASTKEY	= INPUT_PAGE + 1
INPUT_BTN_UP	= INPUT_PAGE + 2
INPUT_BTN_DN	= INPUT_PAGE + 3
INPUT_BTN_LT	= INPUT_PAGE + 4
INPUT_BTN_RT	= INPUT_PAGE + 5
INPUT_BTN0	= INPUT_PAGE + 6
INPUT_BTN1	= INPUT_PAGE + 7

	.global	input_init
	.global	input_poll
	.global	input_wait
	.global	input_publish

	.endif	; _LIB_INPUT_INC_
