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
;   Wire protocol (server -> //c):
;     printable ASCII                  - normal text via COUT (bit 7 set)
;     ESC '=' (32+row) (32+col)        - VT52 cursor address
;     ESC 'L' len_lo len_hi <bytes>    - load `len` bytes to $0A00 then
;                                        JMP $0A00 (DHGR module upload)
;     ESC 'M' <byte>                   - print one raw byte at the
;                                        current cursor (bypasses COUT
;                                        so MouseText codes $40-$5F
;                                        render correctly under the
;                                        alt charset enabled at boot)
;     ESC 'X'                          - bail to Applesoft cold start
;
;   Build:  ca65 rustiic_term.s -o rustiic_term.o
;           ld65 -C rustiic_term.cfg rustiic_term.o -o rustiic_term.bin
; ============================================================================

		.setcpu	"65C02"

; ---- Apple //c IO ----------------------------------------------------------
KBD		=	$C000		; keyboard data + strobe (bit7)
KBDSTRB		=	$C010		; clear keyboard strobe
SOLID_APL	=	$C062		; closed-apple button (bit7 = pressed)
SETALTCHAR	=	$C00F		; write enables alt char set (MouseText $40-$5F)
CLR80VID	=	$C054		; PAGE2 off  (writes $0400-$07FF -> main bank)
SET80VID	=	$C055		; PAGE2 on   (writes $0400-$07FF -> aux bank)

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

		; Enable the alternate character set so VRAM bytes $40-$5F
		; render as MouseText glyphs (corners, lines, mouse, apples).
		; The plain rx path still goes through COUT which forces bit 7
		; on, so normal ASCII output is unaffected; MouseText reaches
		; the screen only via the ESC 'M' opcode below, which writes
		; raw bytes directly to text-page memory.
		STA	SETALTCHAR

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
BASL		=	$0028		; current row text-page base (set by VTAB)
VTAB		=	$FC22
LOAD_DEST	=	$0A00		; ESC L destination (DHGR module entry)

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
		JMP	set_state
:		CMP	#$4C		; 'L'
		BNE	:+
		; init load destination + clear length high
		LDA	#<LOAD_DEST
		STA	load_tgt+1
		LDA	#>LOAD_DEST
		STA	load_tgt+2
		LDA	#$04
		JMP	set_state
:		CMP	#$4D		; 'M' -- raw byte to screen at cursor
		BNE	:+
		LDA	#$07
		JMP	set_state
:		CMP	#$58		; 'X' -- server-initiated bail to BASIC
		BEQ	bail
		STZ	rx_state
		RTS
st2:		CPX	#$02		; ESC = row
		BNE	st3
		SEC
		SBC	#$20
		STA	rx_row
		LDA	#$03
		JMP	set_state
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
		JMP	set_state
st5:		CPX	#$05		; ESC L len_hi
		BNE	st6_or_st7
		STA	load_len+1
		LDA	#$06
		JMP	set_state
st6_or_st7:	CPX	#$06
		BNE	st7
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
; ESC M <byte>: write `byte` straight into 80-col text-page memory at
; the current firmware cursor (CV/OURCH).  The 80-col text page is
; interleaved -- even columns live in aux bank, odd columns in main
; -- so we toggle PAGE2 ($C054/$C055) around the store for even cols.
; The byte is taken at face value (no bit-7 OR), letting the server
; place MouseText codes ($40-$5F under alt charset) directly.
;
; Only the cursor's *column* advances; vertical motion remains the
; server's job (via ESC '=' / explicit CR).
; ----------------------------------------------------------------------------
st7:
		PHA			; save the raw byte
		JSR	VTAB		; refresh BASL/BASH for current row (CV)
		LDA	OURCH
		LSR			; carry = original bit 0 (1 = odd col)
		TAY			; Y = col / 2 (byte offset into row)
		BCS	@odd
		; even col -> aux bank: enable PAGE2, store, restore PAGE2 off
		STA	SET80VID
		PLA
		STA	(BASL),Y
		STA	CLR80VID
		BRA	@done
@odd:
		PLA
		STA	(BASL),Y
@done:
		INC	OURCH
		STZ	rx_state
		RTS

; ----------------------------------------------------------------------------
; Scratch storage in this sector.
; ----------------------------------------------------------------------------
scratch:	.byte	$00
rx_state:	.byte	$00
rx_row:		.byte	$00
load_len:	.word	$0000
