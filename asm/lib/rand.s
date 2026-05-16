; ============================================================================
; lib/rand.s -- 16-bit Galois LFSR PRNG.
;
; Period: 65535 (skips $0000). Polynomial: x^16 + x^14 + x^13 + x^11 + 1
; (taps mask $B400). Cheap (~14 cycles per byte) and good enough for
; gameplay RNG (enemy pick, damage roll, map seed).
;
; Seeded at engine_init from a fixed constant; later we'll mix in
; KBD strobe / VBL counter once those exist.
;
; ABI:
;   $0300 JT_RNG_NEXT  -> ARG0 = next byte           (carry clear)
;   $0303 JT_RNG_RANGE : ARG0 = max (1..255)
;                     -> ARG0 = result in [0, max)   (carry clear)
;                        if max=0, ARG0=0           (carry SET = error)
;
; Clobbers: A. Preserves X, Y, $20-$2F (apart from ARG0).
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"rand.inc"

		.export	rand_init
		.export	rand_seed
		.export	rand_next_u8
		.export	rand_range_u8

; ----- private state (BSS in main RAM, not ZP) ------------------------------
		.segment "BSS"
rand_state_lo:	.res	1
rand_state_hi:	.res	1

		.segment "CODE"

; ----------------------------------------------------------------------------
; rand_init -- seed state, patch $0300 slots.
; Called from engine_init after jumptable_install.
; Clobbers A, X.
; ----------------------------------------------------------------------------
		.proc	rand_init
		LDA	#$3C		; arbitrary non-zero seed
		LDX	#$A7
		JSR	rand_seed
		JT_SET_SLOT JT_RNG_NEXT,  jt_rng_next
		JT_SET_SLOT JT_RNG_RANGE, jt_rng_range
		RTS
		.endproc

; ----------------------------------------------------------------------------
; rand_seed -- A=lo, X=hi.  Refuses $0000 (forces $0001).
; ----------------------------------------------------------------------------
		.proc	rand_seed
		STA	rand_state_lo
		STX	rand_state_hi
		ORA	rand_state_hi
		BNE	@ok
		LDA	#$01
		STA	rand_state_lo
@ok:		RTS
		.endproc

; ----------------------------------------------------------------------------
; rand_next_u8 -- advance LFSR one step, return low byte in A.
; Galois LFSR: shift right; if bit shifted out was 1, XOR state with $B400.
; Clobbers A. Preserves X, Y.
; ----------------------------------------------------------------------------
		.proc	rand_next_u8
		LSR	rand_state_hi
		ROR	rand_state_lo
		BCC	@done
		LDA	rand_state_hi
		EOR	#$B4
		STA	rand_state_hi
@done:		LDA	rand_state_lo
		RTS
		.endproc

; ----------------------------------------------------------------------------
; rand_range_u8 -- A = max.  Returns A in [0, max) via rejection sampling
; against the next power of two >= max. Cheap and unbiased.
;
; max=0 -> A=0, carry SET (error).
; max=1 -> A=0.
; ----------------------------------------------------------------------------
		.proc	rand_range_u8
		CMP	#0
		BNE	@nz
		SEC
		RTS
@nz:		STA	@max + 1	; self-mod operand
		; Build mask = next-power-of-two-1 >= max-1
		; Iterate: start mask=$01, while mask < max-1, shift+1
		DEC	@max + 1
		LDA	@max + 1
		CMP	#0		; max was 1 -> return 0
		BNE	@build
		LDA	#0
		CLC
		RTS
@build:		LDA	#$01
@grow:		CMP	@max + 1
		BCS	@have_mask
		ASL
		ORA	#$01
		BNE	@grow		; always taken
@have_mask:	STA	@mask + 1
		INC	@max + 1	; restore max
@retry:		JSR	rand_next_u8
@mask:		AND	#$00		; patched
		CMP	@max + 1
		BCS	@retry
		CLC
		RTS
@max:		.byte	0
		.endproc

; ----------------------------------------------------------------------------
; jt_rng_next -- $0300 ABI body for JT_RNG_NEXT.
; ----------------------------------------------------------------------------
		.proc	jt_rng_next
		JSR	rand_next_u8
		STA	ARG0
		CLC
		RTS
		.endproc

; ----------------------------------------------------------------------------
; jt_rng_range -- $0300 ABI body for JT_RNG_RANGE.
; In: ARG0 = max
; Out: ARG0 = result, carry SET if max=0.
; ----------------------------------------------------------------------------
		.proc	jt_rng_range
		LDA	ARG0
		JSR	rand_range_u8
		STA	ARG0
		RTS
		.endproc
