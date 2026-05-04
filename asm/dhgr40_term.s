; ============================================================================
; dhgr40_term.s -- 40-column DHGR true-color terminal module.
;
;   Interactive: pumps SCC modem rx -> screen, keyboard -> SCC tx,
;   exits on closed-apple.  Speaks the same minimal VT52-ish protocol
;   as the boot terminal so the host-side pump() can drive it via
;   the existing diff renderer:
;
;     0x08              backspace        (col-- clamp)
;     0x0A              line feed        (row++ wrap)
;     0x0C              form feed        (clear screen)
;     0x0D              carriage ret     (col = 0)
;     0x1B '=' r c      cursor address   (row=r-32, col=c-32)
;     0x1B 'F' xy       set color        (fg=hi nibble, bg=lo nibble)
;     0x1B 'C'          clear screen
;     0x1B 'X'          exit module
;     0x20..0x7E        printable        (plot at cursor, col++)
;
;   Loaded into $0900 by the boot terminal's `ESC L`.
; ============================================================================

		.setcpu	"65C02"

KBD		=	$C000
KBDSTRB		=	$C010
SOLID_APL	=	$C062
RDVBLBAR	=	$C019

SCC_DATA	=	$C0A8
SCC_STAT	=	$C0A9

SS_80STORE_ON	=	$C001
SS_80COL_OFF	=	$C00C
SS_80COL_ON	=	$C00D
SS_GRAPHICS	=	$C050
SS_TEXT		=	$C051
SS_FULLSCREEN	=	$C052
SS_PAGE1	=	$C054
SS_PAGE2	=	$C055
SS_HIRES_OFF	=	$C056
SS_HIRES_ON	=	$C057
SS_DHIRES_ON	=	$C05E
SS_DHIRES_OFF	=	$C05F

ZP_FNT		=	$06
ZP_FNT_HI	=	$07

COL_BLACK	=	0
COL_WHITE	=	15

COLS		=	40
ROWS		=	24

DUTY_TEXT	=	1
DUTY_COLOR	=	1

		.segment "DHGR"

; ============================================================================
; Entry.
; ============================================================================
start:
		SEI
		CLD
		LDX	#$FF
		TXS

		JSR	dhgr_init
		JSR	clear_text
		JSR	dhgr_clear

		LDA	#0
		STA	cur_row
		STA	cur_col
		STA	rx_state
		STA	frame_ctr
		LDA	#$F0		; default: white fg, black bg
		STA	color_byte
		LDA	#$80
		STA	attr

main_loop:
wait_active:
		LDA	RDVBLBAR
		BMI	wait_active
wait_vbl:
		LDA	RDVBLBAR
		BPL	wait_vbl

		LDA	frame_ctr
		CMP	#DUTY_TEXT
		BCS	show_color
		BIT	SS_TEXT
		STA	SS_80COL_OFF
		BRA	tick
show_color:
		BIT	SS_GRAPHICS
		STA	SS_80COL_ON
tick:
		INC	frame_ctr
		LDA	frame_ctr
		CMP	#(DUTY_TEXT + DUTY_COLOR)
		BCC	pump
		LDA	#0
		STA	frame_ctr

pump:
@rx_loop:
		LDA	SCC_STAT
		AND	#$08		; RDRF
		BEQ	no_rx
		LDA	SCC_DATA
		JSR	rx_byte
		BRA	@rx_loop
no_rx:
		LDA	KBD
		BPL	no_kb
		AND	#$7F
		STA	scratch
		STA	KBDSTRB
		CMP	#$1D		; Ctrl-] -> exit
		BEQ	bail
@tx_wait:
		LDA	SCC_STAT
		AND	#$10		; TDRE
		BEQ	@tx_wait
		LDA	scratch
		STA	SCC_DATA
no_kb:
		JMP	main_loop

bail:
		JSR	dhgr_to_text
		JMP	$0801

; ============================================================================
; rx_byte: serial dispatcher.
; ============================================================================
rx_byte:
		LDX	rx_state
		BNE	dispatch

		; State 0: idle.  Watch for ESC, control codes, or printable.
		CMP	#$1B
		BNE	@not_esc
		LDA	#1
		STA	rx_state
		RTS
@not_esc:
		CMP	#$0D
		BNE	@not_cr
		STZ	cur_col
		RTS
@not_cr:
		CMP	#$0A
		BNE	@not_lf
		JSR	advance_row
		RTS
@not_lf:
		CMP	#$08
		BNE	@not_bs
		LDX	cur_col
		BEQ	@bs_done
		DEC	cur_col
@bs_done:
		RTS
@not_bs:
		CMP	#$0C
		BNE	@not_ff
		JSR	clear_text
		JSR	dhgr_clear
		STZ	cur_row
		STZ	cur_col
		RTS
@not_ff:
		; Printable?
		CMP	#$20
		BCS	@p1
		RTS
@p1:		CMP	#$7F
		BCC	@p2
		RTS
@p2:		JSR	plot_glyph
		INC	cur_col
		LDA	cur_col
		CMP	#COLS
		BCS	@wrap
		RTS
@wrap:
		STZ	cur_col
		JMP	advance_row

dispatch:
		CPX	#1
		BNE	@d2
		; State 1: ESC opcode.
		CMP	#'='
		BNE	@e_notcur
		LDA	#2
		STA	rx_state
		RTS
@e_notcur:
		CMP	#'F'
		BNE	@e_notfc
		LDA	#4
		STA	rx_state
		RTS
@e_notfc:
		CMP	#'C'
		BNE	@e_notc
		JSR	clear_text
		JSR	dhgr_clear
		STZ	cur_row
		STZ	cur_col
		STZ	rx_state
		RTS
@e_notc:
		CMP	#'X'
		BNE	@e_other
		JSR	dhgr_to_text
		JMP	$0801
@e_other:
		STZ	rx_state
		RTS

@d2:
		CPX	#2
		BNE	@d3
		; State 2: ESC = row.
		SEC
		SBC	#32
		CMP	#ROWS
		BCC	@row_ok
		LDA	#0
@row_ok:
		STA	cur_row
		LDA	#3
		STA	rx_state
		RTS

@d3:
		CPX	#3
		BNE	@d4
		; State 3: ESC = col.
		SEC
		SBC	#32
		CMP	#COLS
		BCC	@col_ok2
		LDA	#0
@col_ok2:
		STA	cur_col
		STZ	rx_state
		RTS

@d4:
		; State 4: ESC F color.
		STA	color_byte
		STZ	rx_state
		RTS

rx_drop:
		RTS

; ============================================================================
; advance_row: cur_row++, wrap to 0 at ROWS (no scroll yet).
; ============================================================================
advance_row:
		INC	cur_row
		LDA	cur_row
		CMP	#ROWS
		BCC	@ar_done
		STZ	cur_row
@ar_done:
		RTS

; ============================================================================
; Soft-switches.
; ============================================================================
dhgr_init:
		STA	SS_80STORE_ON
		STA	SS_80COL_ON
		BIT	SS_FULLSCREEN
		BIT	SS_HIRES_ON
		BIT	SS_DHIRES_ON
		BIT	SS_GRAPHICS
		RTS

dhgr_to_text:
		BIT	SS_PAGE1
		BIT	SS_DHIRES_OFF
		BIT	SS_HIRES_OFF
		STA	SS_80COL_OFF
		BIT	SS_TEXT
		RTS

; ============================================================================
; clear_text: fill 40-col text page MAIN ($0400-$07FF) with spaces.
; ============================================================================
clear_text:
		BIT	SS_PAGE1
		LDA	#$00
		STA	tx_st+1
		LDA	#$04
		STA	tx_st+2
tx_page:
		LDA	#$A0
		LDY	#$00
tx_byte:
tx_st:		STA	$0400,Y
		INY
		BNE	tx_byte
		INC	tx_st+2
		LDA	tx_st+2
		CMP	#$08
		BNE	tx_page
		RTS

; ============================================================================
; dhgr_clear: zero $2000-$3FFF in both AUX and MAIN.
; ============================================================================
dhgr_clear:
		BIT	SS_PAGE2
		JSR	clear_8k
		BIT	SS_PAGE1
		JSR	clear_8k
		RTS

clear_8k:
		LDA	#$00
		STA	clr_st+1
		LDA	#$20
		STA	clr_st+2
clr_page:
		LDA	#$00
		LDY	#$00
clr_byte:
clr_st:		STA	$2000,Y
		INY
		BNE	clr_byte
		INC	clr_st+2
		LDA	clr_st+2
		CMP	#$40
		BNE	clr_page
		RTS

; ============================================================================
; plot_glyph: render ASCII char A at cur_row,cur_col.
;   - Text-page MAIN: write glyph (with attr) so the //c character ROM
;     paints a sharp white-on-black 14-px-wide cell during the TEXT
;     frame.
;   - DHGR AUX+MAIN: paint a solid bg-color block during the COLOR
;     frame.  The eye averages text+color frames as a colored glyph
;     on a colored field (no chroma fringing on the glyph itself).
; ============================================================================
plot_glyph:
		AND	#$7F
		STA	glyph_char

		; --- TEXT WRITE ($0400 + text_row_hi[r]*256 + row_lo[r] + col, MAIN) ---
		BIT	SS_PAGE1
		LDX	cur_row
		LDA	row_lo,X
		CLC
		ADC	cur_col
		STA	tw_st+1
		LDA	text_row_hi,X
		ADC	#$00
		STA	tw_st+2
		LDA	glyph_char
		ORA	attr
tw_st:		STA	$0400

		; --- DHGR CELL ADDRESS ($2000 + row_hi[r]*256 + row_lo[r] + col) ---
		LDX	cur_row
		LDA	row_lo,X
		CLC
		ADC	cur_col
		STA	cell_base_lo
		LDA	row_hi,X
		ADC	#$00
		STA	cell_base_hi

		; --- DECODE color_byte: lo nibble = bg = block color ---
		LDA	color_byte
		AND	#$0F
		ASL	A
		ASL	A
		TAX
		LDA	solid_table,X
		STA	fg_aux			; phase0 -> AUX
		LDA	solid_table+3,X
		STA	fg_main			; phase3 -> MAIN
		LDA	solid_table+2,X		; (unused now)
		STA	bg_aux
		LDA	solid_table+1,X
		STA	bg_main

		LDY	#$00
sline_loop:
		PHY
		TYA
		ASL	A
		ASL	A
		CLC
		ADC	cell_base_hi
		STA	auxw_st+2
		STA	mainw_st+2
		LDA	cell_base_lo
		STA	auxw_st+1
		STA	mainw_st+1

		BIT	SS_PAGE2
		LDA	fg_aux
auxw_st:	STA	$2000

		BIT	SS_PAGE1
		LDA	fg_main
mainw_st:	STA	$2000

		PLY
		INY
		CPY	#$08
		BEQ	sline_done
		JMP	sline_loop
sline_done:
		RTS

; ============================================================================
; Tables.
; ============================================================================
row_lo:
		.byte	$00,$80,$00,$80,$00,$80,$00,$80
		.byte	$28,$A8,$28,$A8,$28,$A8,$28,$A8
		.byte	$50,$D0,$50,$D0,$50,$D0,$50,$D0
row_hi:
		.byte	$20,$20,$21,$21,$22,$22,$23,$23
		.byte	$20,$20,$21,$21,$22,$22,$23,$23
		.byte	$20,$20,$21,$21,$22,$22,$23,$23
text_row_hi:
		.byte	$04,$04,$05,$05,$06,$06,$07,$07
		.byte	$04,$04,$05,$05,$06,$06,$07,$07
		.byte	$04,$04,$05,$05,$06,$06,$07,$07

solid_table:
		.byte	$00,$00,$00,$00		; black
		.byte	$11,$08,$44,$22		; magenta
		.byte	$22,$11,$08,$44		; brown
		.byte	$33,$19,$4C,$66		; orange
		.byte	$44,$22,$11,$08		; dark green
		.byte	$55,$2A,$55,$2A		; grey1
		.byte	$66,$33,$19,$4C		; green
		.byte	$77,$3B,$5D,$6E		; yellow
		.byte	$08,$44,$22,$11		; dark blue
		.byte	$19,$4C,$66,$33		; violet
		.byte	$2A,$55,$2A,$55		; grey2
		.byte	$3B,$5D,$6E,$77		; pink
		.byte	$4C,$66,$33,$19		; medium blue
		.byte	$5D,$6E,$77,$3B		; light blue
		.byte	$6E,$77,$3B,$5D		; aqua
		.byte	$7F,$7F,$7F,$7F		; white

; ============================================================================
; State.
; ============================================================================
cur_row:	.byte	0
cur_col:	.byte	0
glyph_char:	.byte	0
cell_base_lo:	.byte	0
cell_base_hi:	.byte	0
font_byte:	.byte	0
fg_color:	.byte	COL_WHITE
bg_color:	.byte	COL_BLACK
fg_aux:		.byte	0
fg_main:	.byte	0
bg_aux:		.byte	0
bg_main:	.byte	0
out_aux:	.byte	0
out_main:	.byte	0
tmp:		.byte	0
tmp2:		.byte	0
rx_state:	.byte	0
color_byte:	.byte	$F0		; fg=white, bg=black
scratch:	.byte	0
attr:		.byte	$80		; text-page attr (normal)
frame_ctr:	.byte	0

		.align	256
font:
		.incbin	"../assets/font.bin"
