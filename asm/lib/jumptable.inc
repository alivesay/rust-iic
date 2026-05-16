; ============================================================================
; lib/jumptable.inc -- $0300 ABI extern decls + slot constants.
;
; ============================================================================

	.ifndef _LIB_JUMPTABLE_INC_
_LIB_JUMPTABLE_INC_	= 1

	.global	jumptable_install

; ----- public ABI slots ----------------------------------------------------
JT_BASE		= $0300

JT_TILE_DRAW	= JT_BASE + $00		; $0300
JT_TILE_SET_MAP	= JT_BASE + $03
JT_MAP_DRAW_ALL	= JT_BASE + $06
JT_MAP_DRAW_DIRTY = JT_BASE + $09
JT_MAP_SCROLL	= JT_BASE + $0C
JT_SPRITE_DRAW	= JT_BASE + $0F
JT_SPRITE_CLEAR	= JT_BASE + $12
JT_SFX_PLAY	= JT_BASE + $15
JT_MUSIC_START	= JT_BASE + $18
JT_MUSIC_STOP	= JT_BASE + $1B
JT_MUSIC_TICK	= JT_BASE + $1E
JT_HGR_CLEAR	= JT_BASE + $21
JT_HGR_PIXEL	= JT_BASE + $24
JT_HGR_LINE	= JT_BASE + $27
JT_TEXT_DRAW_CHAR = JT_BASE + $2A
JT_TEXT_DRAW_STR = JT_BASE + $2D
JT_RNG_NEXT	= JT_BASE + $30
JT_RNG_RANGE	= JT_BASE + $33
JT_WAIT_VBL	= JT_BASE + $36
JT_MODE_SWAP	= JT_BASE + $39
JT_WIN_OPEN	= JT_BASE + $3C
JT_WIN_CLOSE	= JT_BASE + $3F

; ----- helper slots (BASIC perf shortcuts) ---------------------------------
JT_MAP_FILL_RECT = JT_BASE + $42	; ARG0=tile, ARG1=x0, ARG2=y0, ARG3=x1, ARG4=y1
JT_MAP_BORDER	= JT_BASE + $45		; ARG0=border tile, ARG1=interior tile
JT_TILE_MOVE	= JT_BASE + $48		; ARG0=bg, ARG1=ox, ARG2=oy, ARG3=spr, ARG4=nx, ARG5=ny
JT_INPUT_DXDY	= JT_BASE + $4B		; out: ARG0=dx (-1/0/1), ARG1=dy (-1/0/1)

; ----- reserved (was JT_CART_LOAD / JT_CART_SPLASH; cart concept removed)
JT_RESERVED_4E	= JT_BASE + $4E		; reserved (future map_load)
JT_RESERVED_51	= JT_BASE + $51		; reserved (future map_save)

; ----- map query helpers ---------------------------------------------------
JT_TILE_AT	= JT_BASE + $54		; ARG0=col, ARG1=row
					; -> A := map[row*20 + col]; ARG0 := A

; ----- editor helpers ------------------------------------------------------
JT_TILE_EDIT_BLIT   = JT_BASE + $57	; ARG0=tile_id
					; -> render tile zoomed 7x at HGR
					;    (col_byte=2, row=0), 98x112 px.
JT_TILE_EDIT_CURSOR = JT_BASE + $5A	; ARG0=col(0..13), ARG1=row(0..15)
					; -> XOR a 7x7 inverse block onto
					;    the editor canvas at that cell.
JT_TILES_STRIP_CURSOR = JT_BASE + $5D	; ARG0=col, ARG1=row (in map grid)
					; -> XOR a 14x16 highlight box at
					;    map cell (col,row).

; ----- multi-layer map helpers (layer 0..2; tile 0 = transparent) ----------
JT_TILE_SET_MAP_L = JT_BASE + $60	; ARG0=tile, ARG1=wcol, ARG2=wrow, ARG3=layer
JT_TILE_AT_L	 = JT_BASE + $63	; ARG0=wcol, ARG1=wrow, ARG2=layer
					; -> ARG0 := map[layer][wrow*W + wcol]

; ----- viewport / resize ---------------------------------------------------
JT_MAP_SET_CAM	= JT_BASE + $66		; ARG0=cam_x, ARG1=cam_y
					; -> set camera, mark all viewport dirty
JT_MAP_RESIZE	= JT_BASE + $69		; ARG0=W, ARG1=H
					; -> resize map (zeros data); carry=1 on bad

; First free slot ($03BF cap)
JT_NEXT_FREE	= JT_BASE + $6C

; ----- slot-patch macro ----------------------------------------------------
; JT_SET_SLOT  slot_addr, target_proc
;
; Rewrites the JMP target in the slot from jt_unimpl (or whatever) to
; `target_proc`. Use from each lib's init routine after jumptable_install
; has populated the table.
;
; Clobbers A. Does not touch X, Y.
.macro	JT_SET_SLOT	slot, target
		LDA	#<(target)
		STA	slot+1
		LDA	#>(target)
		STA	slot+2
.endmacro

	.endif	; _LIB_JUMPTABLE_INC_
