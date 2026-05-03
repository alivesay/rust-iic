; ============================================================================
; dhgr_term.s -- 80-col text + DHGR color flicker terminal module.
;
;   Interactive: pumps SCC modem rx -> screen, keyboard -> SCC tx,
;   exits on closed-apple.  Speaks the same VT52-ish protocol as the
;   40-col module:
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
;   Display: VBL-synced 1:1 alternation of TEXT (80-col $0400) and
;   GRAPHICS (DHGR $2000 color blocks).  SCC poll happens once per
;   frame in the flicker loop; keyboard poll likewise.
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

COL_BLACK	=	0
COL_WHITE	=	15

COLS		=	80
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

		JSR	mode_init
		JSR	clear_text
		JSR	clear_dhgr

		LDA	#0
		STA	cur_row
		STA	cur_col
		STA	rx_state
		STA	frame_ctr
		LDA	#$F0		; default white-on-black
		STA	color_byte
		LDA	#$80
		STA	attr

; ============================================================================
; Main loop: VBL-synced flicker AND SCC/kbd pump.
; ============================================================================
flip_loop:
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
		BRA	tick
show_color:
		BIT	SS_GRAPHICS
tick:
		INC	frame_ctr
		LDA	frame_ctr
		CMP	#(DUTY_TEXT + DUTY_COLOR)
		BCC	pump
		LDA	#0
		STA	frame_ctr

pump:
		; Drain SCC fully each frame (BBS bursts faster than 60 Hz).
@rx_loop:
		LDA	SCC_STAT
		AND	#$08
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
		CMP	#$1D		; Ctrl-] -> exit to boot terminal
		BEQ	bail
@tx_wait:
		LDA	SCC_STAT
		AND	#$10
		BEQ	@tx_wait
		LDA	scratch
		STA	SCC_DATA
no_kb:
		JMP	flip_loop

bail:
		BIT	SS_TEXT
		BIT	SS_DHIRES_OFF
		BIT	SS_HIRES_OFF
		BIT	SS_PAGE1
		JMP	$0801

; ============================================================================
; rx_byte: serial dispatcher.
; ============================================================================
rx_byte:
		LDX	rx_state
		BNE	dispatch

		; State 0: idle.
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
		JSR	clear_dhgr
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
		JSR	clear_dhgr
		STZ	cur_row
		STZ	cur_col
		STZ	rx_state
		RTS
@e_notc:
		CMP	#'X'
		BNE	@e_other
		BIT	SS_TEXT
		BIT	SS_DHIRES_OFF
		BIT	SS_HIRES_OFF
		BIT	SS_PAGE1
		JMP	$0801
@e_other:
		STZ	rx_state
		RTS

@d2:
		CPX	#2
		BNE	@d3
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
		STA	color_byte
		STZ	rx_state
		RTS

; ============================================================================
; advance_row: cur_row++, wrap.
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
; mode_init.
; ============================================================================
mode_init:
		BIT	SS_80COL_ON
		BIT	SS_80STORE_ON
		BIT	SS_FULLSCREEN
		BIT	SS_HIRES_ON
		BIT	SS_DHIRES_ON
		BIT	SS_TEXT
		RTS

; ============================================================================
; clear_text: fill 80-col text page 1 ($0400-$07FF) AUX+MAIN with spaces.
; ============================================================================
clear_text:
		BIT	SS_PAGE2
		JSR	fill_text
		BIT	SS_PAGE1
		JSR	fill_text
		RTS

fill_text:
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
; clear_dhgr: zero $2000-$3FFF AUX+MAIN.
; ============================================================================
clear_dhgr:
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
; plot_glyph: A = ASCII char.
;   - Write text byte (with attr) to text page.
;   - Write fg color block to DHGR cell.
; ============================================================================
plot_glyph:
		STA	glyph_char

		; --- TEXT WRITE ---
		LDX	cur_row
		LDA	row_lo,X
		STA	tw_st+1
		LDA	text_row_hi,X
		STA	tw_st+2
		LDA	cur_col
		LSR	A
		CLC
		ADC	tw_st+1
		STA	tw_st+1
		BCC	tw_no_inc
		INC	tw_st+2
tw_no_inc:
		LDA	cur_col
		AND	#$01
		BEQ	tw_aux
		BIT	SS_PAGE1
		BRA	tw_write
tw_aux:		BIT	SS_PAGE2
tw_write:
		LDA	glyph_char
		AND	#$7F
		ORA	attr
tw_st:		STA	$0400

		; --- DHGR COLOR BLOCK ---
		LDX	cur_row
		LDA	row_lo,X
		STA	cell_base_lo
		LDA	row_hi,X
		STA	cell_base_hi
		LDA	cur_col
		LSR	A
		CLC
		ADC	cell_base_lo
		STA	cell_base_lo
		BCC	pg_no_inc
		INC	cell_base_hi
pg_no_inc:
		LDA	cur_col
		AND	#$01
		STA	cell_parity

		; cell_phase = (col*3) & 3
		LDA	cur_col
		STA	tmp
		ASL	A
		CLC
		ADC	tmp
		AND	#$03
		STA	cell_phase

		; Decode color_byte (lo nibble = bg, used as DHGR block color
		; behind the text-page glyph during the color frame).
		LDA	color_byte
		AND	#$0F
		ASL	A
		ASL	A
		ORA	cell_phase
		TAX
		LDA	solid_table,X
		STA	fg_pat

		LDA	cell_parity
		BEQ	dg_aux
		BIT	SS_PAGE1
		BRA	dg_bank_done
dg_aux:		BIT	SS_PAGE2
dg_bank_done:

		LDY	#$00
sline_loop:
		PHY
		TYA
		ASL	A
		ASL	A
		CLC
		ADC	cell_base_hi
		STA	dst_st+2
		LDA	cell_base_lo
		STA	dst_st+1
		LDA	fg_pat
dst_st:		STA	$2000
		PLY
		INY
		CPY	#$08
		BEQ	@sl_done
		JMP	sline_loop
@sl_done:
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
cell_parity:	.byte	0
cell_phase:	.byte	0
fg_pat:		.byte	0
attr:		.byte	$80
tmp:		.byte	0
frame_ctr:	.byte	0
rx_state:	.byte	0
color_byte:	.byte	$F0
scratch:	.byte	0
