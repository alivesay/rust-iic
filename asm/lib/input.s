; ============================================================================
; lib/input.s
;
; input_poll / input_wait: thin //c keyboard wrappers.
; input_publish: writes the $0260 input page
;   Called once per frame from engine_frame_begin.
; input_init: clear page; nothing in $0300 to wire
; ============================================================================

		.setcpu	"65C02"
		.include	"softswitches.inc"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"input.inc"

		.export	input_init
		.export	input_poll
		.export	input_wait
		.export	input_publish

		.segment "CODE"

; ----------------------------------------------------------------------------
; input_init -- clear the $0260 input page.
; Clobbers A, X.
; ----------------------------------------------------------------------------
		.proc	input_init
		LDA	#0
		LDX	#7
@l:		STA	INPUT_PAGE,X
		DEX
		BPL	@l
		JT_SET_SLOT JT_INPUT_DXDY, jt_input_dxdy
		RTS
		.endproc

; ----------------------------------------------------------------------------
; input_poll
;
; Returns:
;   A = ASCII key code with high bit stripped, or 0 if no key pending.
;   Z flag set iff no key.
; Clobbers: A. Preserves X, Y.
; ----------------------------------------------------------------------------
		.proc	input_poll
		LDA	KBD
		BPL	@none		; bit7=0 -> no key
		STA	KBDSTRB		; consume
		AND	#$7F
		RTS
@none:
		LDA	#0
		RTS
		.endproc

; ----------------------------------------------------------------------------
; input_wait -- block until a key is pressed
;
; Returns:
;   A = ASCII key code with high bit stripped (always non-zero).
; Clobbers: A. Preserves X, Y.
; ----------------------------------------------------------------------------
		.proc	input_wait
@spin:
		LDA	KBD
		BPL	@spin
		STA	KBDSTRB
		AND	#$7F
		RTS
		.endproc

; ----------------------------------------------------------------------------
; input_publish -- write per-frame input snapshot to $0260.
; Clobbers A, X. Preserves Y.
; ----------------------------------------------------------------------------
		.proc	input_publish
		; --- clear the per-frame one-shots
		LDA	#0
		STA	INPUT_ANYKEY
		STA	INPUT_BTN_UP
		STA	INPUT_BTN_DN
		STA	INPUT_BTN_LT
		STA	INPUT_BTN_RT

		; --- sample game-port buttons (bit 7 of OPN_APL/SOLID_APL)
		LDA	OPN_APL
		AND	#$80
		STA	INPUT_BTN0
		LDA	SOLID_APL
		AND	#$80
		STA	INPUT_BTN1

		; --- check keyboard
		LDA	KBD
		BPL	@done		; no strobe, done
		STA	KBDSTRB		; consume strobe
		AND	#$7F
		STA	INPUT_LASTKEY
		LDX	#$80
		STX	INPUT_ANYKEY

		; --- decode direction (IJKL primary, arrows secondary)
		CMP	#'I'
		BEQ	@up
		CMP	#'i'
		BEQ	@up
		CMP	#$0B		; up arrow
		BEQ	@up
		CMP	#'K'
		BEQ	@dn
		CMP	#'k'
		BEQ	@dn
		CMP	#$0A		; down arrow
		BEQ	@dn
		CMP	#'J'
		BEQ	@lt
		CMP	#'j'
		BEQ	@lt
		CMP	#$08		; left arrow
		BEQ	@lt
		CMP	#'L'
		BEQ	@rt
		CMP	#'l'
		BEQ	@rt
		CMP	#$15		; right arrow
		BEQ	@rt
		BRA	@done
@up:		STX	INPUT_BTN_UP
		BRA	@done
@dn:		STX	INPUT_BTN_DN
		BRA	@done
@lt:		STX	INPUT_BTN_LT
		BRA	@done
@rt:		STX	INPUT_BTN_RT
@done:		RTS
		.endproc

; ----------------------------------------------------------------------------
; jt_input_dxdy -- pack the four directional one-shots into (dx, dy).
;
; Out: ARG0 = dx in {-1, 0, 1} (LT=-1, RT=+1)
;      ARG1 = dy in {-1, 0, 1} (UP=-1, DN=+1)
; Stored as 2's-complement bytes ($FF for -1) so BASIC can do
;   DX = PEEK(32): IF DX > 127 THEN DX = DX - 256
; or just  PX = PX + (DX = 1) - (DX = 255).
; ----------------------------------------------------------------------------
		.proc	jt_input_dxdy
		LDA	#0
		LDX	INPUT_BTN_LT
		BEQ	@nx_lt
		LDA	#$FF		; -1
@nx_lt:		LDX	INPUT_BTN_RT
		BEQ	@nx_rt
		LDA	#$01
@nx_rt:		STA	ARG0
		LDA	#0
		LDX	INPUT_BTN_UP
		BEQ	@nx_up
		LDA	#$FF
@nx_up:		LDX	INPUT_BTN_DN
		BEQ	@nx_dn
		LDA	#$01
@nx_dn:		STA	ARG1
		CLC
		RTS
		.endproc
