; ============================================================================
; rustiic_term.s -- Boot-sector terminal for the rust-iic virtual BBS
; ----------------------------------------------------------------------------
;   The Apple //c boot ROM at $C600 reads T0/S0 into $0800-$08FF and
;   jumps to $0801.
;
;   The terminal:
;     * polls the SCC channel A modem port (slot 2, $C0A8/$C0A9)
;     * forwards modem rx -> screen, keyboard -> modem tx
;     * exits to BASIC on closed-Apple ($C062 / SOLID-APPLE)
;
;   Build:  ca65 rustiic_term.s -o rustiic_term.o
;           ld65 -C rustiic_term.cfg rustiic_term.o -o rustiic_term.bin
; ============================================================================

		.setcpu	"65C02"

; ---- Apple //c IO ----------------------------------------------------------
KBD		=	$C000		; keyboard data + strobe (bit7)
KBDSTRB		=	$C010		; clear keyboard strobe
SOLID_APL	=	$C062		; closed-apple button (bit7 = pressed)

; SCC Channel A on the //c is the modem port (slot 2 ACIA aliases).
; $C0A8 = data    $C0A9 = status (bit3=RDRF, bit4=TDRE, bit5=DCD-low)
SCC_DATA	=	$C0A8
SCC_STAT	=	$C0A9

; ---- Apple //c monitor entry points ----------------------------------------
COUT		=	$FDED		; output A (bit7 set) via current device
HOME		=	$FC58		; clear screen, cursor home
BASIC		=	$E000		; cold-start Applesoft

		.segment "BOOT"

; Byte at $0800: conventional Disk-II boot sector count.  The //c boot
; ROM ignores this and JMPs to $0801, but keep the convention for any
; tooling that inspects boot blocks.
		.byte	$01

start:
		SEI			; mask IRQs while we settle
		CLD
		LDX	#$FF
		TXS			; reset stack

; ---- activate //c built-in 80-column firmware ------------------------------
;   $C300 is the slot-3 entry point.  On the //c this is the internal
;   80-col card; calling it self-installs as the COUT/KEYIN handler
;   (sets CSWL/H), enables 80STORE, and clears the screen.  Equivalent
;   to typing PR#3 at the BASIC prompt.
		JSR	$C300

		JSR	HOME		; clear screen (now in 80-col)

main_loop:
; ---- modem -> screen -------------------------------------------------------
		LDA	SCC_STAT
		AND	#$08		; RDRF = byte available
		BEQ	no_rx
		LDA	SCC_DATA
		JSR	rx_byte		; raw 8-bit; rx_byte masks for text path
no_rx:
; ---- keyboard -> modem -----------------------------------------------------
		LDA	KBD
		BPL	no_kb		; no key pressed
		AND	#$7F		; strip strobe bit; want plain ASCII
		STA	scratch
		STA	KBDSTRB		; ack
@tx_wait:
		LDA	SCC_STAT
		AND	#$10		; TDRE
		BEQ	@tx_wait
		LDA	scratch
		STA	SCC_DATA
no_kb:
; ---- closed-apple = exit to BASIC ------------------------------------------
		BIT	SOLID_APL
		BMI	bail
		BRA	main_loop

bail:
		JSR	HOME
		JMP	BASIC

; ----------------------------------------------------------------------------
; rx_byte: serial dispatcher.  Supported sequences:
;   ESC '=' (32+row) (32+col)        - cursor to row,col (VT52)
;   ESC 'L' len_lo len_hi <bytes>    - load `len` bytes to $0900 then
;                                       JMP $0900 (single auto-launch)
; Anything else after ESC is silently dropped.
; ----------------------------------------------------------------------------
OURCH		=	$057B
CV		=	$0025
VTAB		=	$FC22
LOAD_DEST	=	$0900

rx_byte:
		LDX	rx_state
		BNE	dispatch
		CMP	#$1B		; ESC?
		BNE	plain
		LDA	#$01
		STA	rx_state
		RTS
plain:		AND	#$7F
		ORA	#$80
		JMP	COUT

dispatch:
		CPX	#$01
		BNE	st2
		; ESC opcode
		CMP	#$3D		; '='
		BNE	:+
		LDA	#$02
		BRA	set_state
:		CMP	#$4C		; 'L'
		BNE	:+
		; init load destination + clear length high
		LDA	#<LOAD_DEST
		STA	load_tgt+1
		LDA	#>LOAD_DEST
		STA	load_tgt+2
		LDA	#$04
		BRA	set_state
:		STZ	rx_state
		RTS
st2:		CPX	#$02		; ESC = row
		BNE	st3
		SEC
		SBC	#$20
		STA	rx_row
		LDA	#$03
		BRA	set_state
st3:		CPX	#$03		; ESC = row col
		BNE	st4
		SEC
		SBC	#$20
		PHA
		LDA	rx_row
		STA	CV
		JSR	VTAB
		PLA
		STA	OURCH
		STZ	rx_state
		RTS
st4:		CPX	#$04		; ESC L len_lo
		BNE	st5
		STA	load_len
		LDA	#$05
		BRA	set_state
st5:		CPX	#$05		; ESC L len_hi
		BNE	st6
		STA	load_len+1
		LDA	#$06
		BRA	set_state
st6:		; payload byte
load_tgt:	STA	$FFFF		; self-modified
		INC	load_tgt+1
		BNE	:+
		INC	load_tgt+2
:		LDA	load_len
		BNE	:+
		DEC	load_len+1
:		DEC	load_len
		LDA	load_len
		ORA	load_len+1
		BNE	rx_done
		; length exhausted -- jump to loaded code
		STZ	rx_state
		JMP	LOAD_DEST
rx_done:	RTS

set_state:	STA	rx_state
		RTS

; ----------------------------------------------------------------------------
; Scratch storage in this sector.
; ----------------------------------------------------------------------------
scratch:	.byte	$00
rx_state:	.byte	$00
rx_row:		.byte	$00
load_len:	.word	$0000
