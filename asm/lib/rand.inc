; ============================================================================
; lib/rand.inc -- PRNG extern decls.
; ============================================================================

	.ifndef _LIB_RAND_INC_
_LIB_RAND_INC_	= 1

	.global	rand_init		; install $0300 slots; seeds state
	.global	rand_seed		; A=lo, X=hi -> state
	.global	rand_next_u8		; -> A
	.global	rand_range_u8		; A=max -> A in [0, max)

	.endif	; _LIB_RAND_INC_
