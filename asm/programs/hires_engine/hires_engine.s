; ============================================================================
; engine.s -- minimal Apple //c hires game engine skeleton.
;
; Loads at $6000 (BRUN GAME from a DOS 3.3 disk, see mkdisk).
; - Switches to HGR1 fullscreen
; - Draws a small XOR sprite that you can move with the arrow keys
; - ESC quits back to BASIC
;
; Memory map:
;   $06/$07     pointer scratch
;   $08         tmp
;   $09..$0C    sprite state
;   $2000-$3FFF HGR page 1
;   $6000+      this code, then state, then hgr address tables
;
; Built with ca65; linker config places everything in segment GAME at $6000.
; ============================================================================

		.setcpu	"65C02"

		.include	"softswitches.inc"
		.include	"macros.inc"
		.include	"hgr.inc"

		.segment "GAME"

; ----- zero page -------------------------------------------------------------
ZP_PTR		=	$06
ZP_PTR_HI	=	$07
ZP_TMP		=	$08

; ----- constants -------------------------------------------------------------
HGR_BASE	=	$2000
SPRITE_ROWS	=	16
SPRITE_W_BYTES	=	2
HGR_COLS	=	40
HGR_ROWS	=	192
KEY_ESC		=	$1B
KEY_LEFT	=	'J'
KEY_RIGHT	=	'L'
KEY_UP		=	'I'
KEY_DOWN	=	'K'
INIT_X		=	20
INIT_Y		=	96

; ============================================================================
; Entry.
; ============================================================================
		.proc	start
		SEI
		CLD
		LDX	#$FF
		TXS

		JSR	hgr_init
		JSR	map_fill

		LDA	#INIT_X
		STA	plr_x
		STA	old_x
		LDA	#INIT_Y
		STA	plr_y
		STA	old_y

		; first draw
		LDA	plr_y
		LDX	plr_x
		JSR	draw_sprite

main_loop:
		KBD_READ
		BEQ	main_loop

		CMP	#KEY_ESC
		BEQ	bail
		CMP	#KEY_LEFT
		BEQ	mv_left
		CMP	#KEY_RIGHT
		BEQ	mv_right
		CMP	#KEY_UP
		BEQ	mv_up
		CMP	#KEY_DOWN
		BEQ	mv_down
		BRA	main_loop

mv_left:
		LDA	plr_x
		CMP	#SPRITE_W_BYTES
		BCC	main_loop
		SEC
		SBC	#SPRITE_W_BYTES
		STA	plr_x
		BRA	redraw
mv_right:
		LDA	plr_x
		CMP	#(HGR_COLS - 2*SPRITE_W_BYTES + 1)
		BCS	main_loop
		CLC
		ADC	#SPRITE_W_BYTES
		STA	plr_x
		BRA	redraw
mv_up:
		LDA	plr_y
		CMP	#SPRITE_ROWS
		BCC	main_loop
		SEC
		SBC	#SPRITE_ROWS
		STA	plr_y
		BRA	redraw
mv_down:
		LDA	plr_y
		CMP	#(HGR_ROWS - 2*SPRITE_ROWS + 1)
		BCS	main_loop
		CLC
		ADC	#SPRITE_ROWS
		STA	plr_y
		BRA	redraw

redraw:
		; restore bg at the previous position
		LDA	old_y
		LDX	old_x
		JSR	erase_sprite
		; save bg + draw at new position
		LDA	plr_y
		LDX	plr_x
		JSR	draw_sprite
		; remember
		LDA	plr_x
		STA	old_x
		LDA	plr_y
		STA	old_y
		JMP	main_loop

bail:
		TEXT_INIT
		LDA	#$00
		STA	KBDSTRB
		RTS
		.endproc

; ============================================================================
; hgr_init: HGR1, fullscreen, primary page.
; ============================================================================
		.proc	hgr_init
		HGR_INIT
		RTS
		.endproc

; ============================================================================
; hgr_clear: zero $2000-$3FFF.
; ============================================================================
		.proc	hgr_clear
		LDA	#$00
		LDX	#$00
@l:
		STA	$2000,X
		STA	$2100,X
		STA	$2200,X
		STA	$2300,X
		STA	$2400,X
		STA	$2500,X
		STA	$2600,X
		STA	$2700,X
		STA	$2800,X
		STA	$2900,X
		STA	$2A00,X
		STA	$2B00,X
		STA	$2C00,X
		STA	$2D00,X
		STA	$2E00,X
		STA	$2F00,X
		STA	$3000,X
		STA	$3100,X
		STA	$3200,X
		STA	$3300,X
		STA	$3400,X
		STA	$3500,X
		STA	$3600,X
		STA	$3700,X
		STA	$3800,X
		STA	$3900,X
		STA	$3A00,X
		STA	$3B00,X
		STA	$3C00,X
		STA	$3D00,X
		STA	$3E00,X
		STA	$3F00,X
		INX
		BNE	@l
		RTS
		.endproc

; ============================================================================
; map_fill: 14x16 parent-tile checkerboard.  Every other parent tile is
; filled with a diagonal stripe pattern; the rest are black.
;
;   - Parent tile = 14 px (2 bytes) x 16 rows -> 20 x 12 grid.
;   - Stripe pattern: pixel at (col,row) lit iff (col + row) mod 4 < 2.
;     That gives 2-pixel-wide diagonals at period 4, which read as plain
;     white-on-black on both mono and color displays (no chroma fringe).
;   - Pattern repeats every 4 rows, so we precompute one byte pair per
;     `row & 3` (lo = bytes at even col, hi = bytes at odd col).
; ============================================================================
		.proc	map_fill
		LDA	#0
		STA	@row_idx
@row_loop:
		LDX	@row_idx
		LDA	hgr_lo,X
		STA	ZP_PTR
		LDA	hgr_hi,X
		STA	ZP_PTR_HI

		; pat = diag[row & 3]
		LDA	@row_idx
		AND	#3
		TAX
		LDA	@diag_lo,X
		STA	@pat_lo
		LDA	@diag_hi,X
		STA	@pat_hi

		; parent_row_parity = (row >> 4) & 1
		LDA	@row_idx
		LSR
		LSR
		LSR
		LSR
		AND	#1
		STA	@prow_par

		LDY	#0
@col_loop:
		; parity of parent col = (Y >> 1) & 1
		TYA
		LSR
		AND	#1
		EOR	@prow_par
		BEQ	@blank
		; lit tile -- pick lo/hi half by Y parity
		TYA
		AND	#1
		BEQ	@use_lo
		LDA	@pat_hi
		BRA	@store
@use_lo:
		LDA	@pat_lo
		BRA	@store
@blank:
		LDA	#0
@store:
		STA	(ZP_PTR),Y
		INY
		CPY	#40
		BNE	@col_loop

		INC	@row_idx
		LDA	@row_idx
		CMP	#192
		BNE	@row_loop
		RTS

; Diagonal pattern (period 4 in both axes), 2 pixels wide.
; Byte = packed 7-bit hires data; bit 7 (palette) left at 0 for mono phase.
;
;   row 0:  ##..##..##..##  -> lo $33, hi $66
;   row 1:  ...##..##..##.  -> lo $19, hi $33   (high bit drops since col 13 is the last)
;   row 2:  ..##..##..##..  -> lo $4C, hi $19
;   row 3:  .##..##..##..#  -> lo $66, hi $4C
@diag_lo:	.byte	$33, $19, $4C, $66
@diag_hi:	.byte	$66, $33, $19, $4C
@row_idx:	.byte	0
@pat_lo:	.byte	0
@pat_hi:	.byte	0
@prow_par:	.byte	0
		.endproc

; ============================================================================
; draw_sprite: save the 14x16 background under the sprite into bg_buf,
;   then write the sprite bytes on top.  (X = byte column, A = top row).
; erase_sprite: restore bg_buf to (X = byte column, A = top row).
;
; bg_buf layout: 32 bytes, same ordering as `sprite` (row-major, left byte
; then right byte per row).
; ============================================================================
		.proc	draw_sprite
		STA	row
		STX	col
		LDY	#0
@row:
		LDA	row
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	col
		STA	ZP_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ZP_PTR_HI

		; left byte
		PHY
		LDY	#0
		LDA	(ZP_PTR),Y
		PLY
		STA	bg_buf,Y
		LDA	sprite,Y
		PHY
		LDY	#0
		STA	(ZP_PTR),Y
		PLY
		INY

		; right byte
		PHY
		LDY	#1
		LDA	(ZP_PTR),Y
		PLY
		STA	bg_buf,Y
		LDA	sprite,Y
		PHY
		LDY	#1
		STA	(ZP_PTR),Y
		PLY
		INY

		INC	row
		CPY	#(SPRITE_ROWS * SPRITE_W_BYTES)
		BNE	@row
		RTS
row:		.byte	0
col:		.byte	0
		.endproc

		.proc	erase_sprite
		STA	row
		STX	col
		LDY	#0
@row:
		LDA	row
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	col
		STA	ZP_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ZP_PTR_HI

		; left byte
		LDA	bg_buf,Y
		PHY
		LDY	#0
		STA	(ZP_PTR),Y
		PLY
		INY

		; right byte
		LDA	bg_buf,Y
		PHY
		LDY	#1
		STA	(ZP_PTR),Y
		PLY
		INY

		INC	row
		CPY	#(SPRITE_ROWS * SPRITE_W_BYTES)
		BNE	@row
		RTS
row:		.byte	0
col:		.byte	0
		.endproc

bg_buf:
		.res	SPRITE_ROWS * SPRITE_W_BYTES, 0

; ============================================================================
; Sprite: 14 px wide x 16 tall, stored as 16 rows of 2 bytes
; (left byte = pixels 0..6, right byte = pixels 7..13).  Bit 7 of each byte
; left at 0 (mono palette).  All runs are >= 2 px wide so the figure stays
; the same in mono and color.
;
; Visually:                     L byte   R byte
;   row  0:  .....######.....   %0000000 %0001111   00 0F (no, see below)
; (we hand-pack each row as two 7-bit fields)
; ============================================================================
sprite:
		;        cols 0-6     cols 7-13
  		.byte   %1000000, %0000011      ; row  0
                .byte   %1100000, %0000111      ; row  1
                .byte   %1100000, %0000010      ; row  2
                .byte   %1100000, %0000111      ; row  3
                .byte   %1000000, %0000111      ; row  4
                .byte   %1110000, %0000011      ; row  5
                .byte   %1111000, %0000111      ; row  6
                .byte   %1111100, %0001111      ; row  7
                .byte   %1101100, %0001111      ; row  8
                .byte   %1110100, %0010111      ; row  9
                .byte   %1110110, %0010111      ; row 10
                .byte   %1110110, %0001111      ; row 11
                .byte   %0110000, %0001100      ; row 12
                .byte   %0011000, %0001110      ; row 13
                .byte   %0011100, %0011100      ; row 14
                .byte   %0011100, %0000000      ; row 15

; ============================================================================
; Player state.
; ============================================================================
plr_x:		.byte	INIT_X
plr_y:		.byte	INIT_Y
old_x:		.byte	INIT_X
old_y:		.byte	INIT_Y

; ============================================================================
; HGR row -> address tables (see lib/hgr.inc for the math).
; ============================================================================
		.align	256
		HGR_ROW_TABLES
