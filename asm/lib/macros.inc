; ============================================================================
; lib/macros.inc -- generic ca65 helper macros for the //c game engine.
;
; Conventions:
;   * Macros are UPPER_SNAKE in invocations only when they map 1:1 to a
;     hardware concept (e.g. `WAIT_VBL`); helpers are snake_case.
;   * No macro modifies CPU flags except as documented.
;   * 16-bit zero-page pointers are referred to by their lo address; the
;     hi byte is implicitly at lo+1 (standard ca65 idiom).
; ============================================================================

		.feature	leading_dot_in_identifiers

; ----- ss_set: poke a soft switch ------------------------------------------
;
;   ss_set addr        ; addr is a $C0xx soft-switch.
;
; Picks the right opcode automatically:
;   * $C000-$C00F  -> STA addr   (write-only data switches)
;   * everything else -> BIT addr  (read-toggle is the canonical access)
;
; Falls back to BIT for anything outside the I/O page.
; -------------------------------------------------------------------------
.macro ss_set addr
	.if (addr >= $C000) .and (addr < $C010)
		STA	addr
	.else
		BIT	addr
	.endif
.endmacro

; ----- set_ptr16: load a 16-bit pointer into a zero-page slot ---------------
;
;   set_ptr16 ptr, value   ; ptr is the lo zp address, value any 16-bit expr
;
; Clobbers A.
; -------------------------------------------------------------------------
.macro set_ptr16 ptr, value
		LDA	#<(value)
		STA	ptr
		LDA	#>(value)
		STA	ptr+1
.endmacro

; ----- add_ptr16_a: ptr += A (unsigned) ------------------------------------
;
;   add_ptr16_a ptr      ; ptr += A (clears A's high byte path), preserves Y.
;
; Clobbers A and the carry; preserves X, Y.
; -------------------------------------------------------------------------
.macro add_ptr16_a ptr
		CLC
		ADC	ptr
		STA	ptr
		LDA	ptr+1
		ADC	#0
		STA	ptr+1
.endmacro

; ----- WAIT_VBL: block until the start of the next vertical blanking ------
;
;   WAIT_VBL              ; spins on RDVBLBAR ($C019) until 0 -> 1 transition
;
; On //e/c, RDVBLBAR bit 7 reads:
;   * 1 = active scan
;   * 0 = vertical blanking
;
; Clobbers A.
; -------------------------------------------------------------------------
.macro WAIT_VBL
	.local @active, @vbl
@active:
		LDA	RDVBLBAR
		BMI	@active		; wait until we're inside vbl (bit7=0)
@vbl:
		LDA	RDVBLBAR
		BPL	@vbl		; wait until vbl ends (bit7=1)
.endmacro

; ----- HGR_INIT / TEXT_INIT: standard mode setup ---------------------------
.macro HGR_INIT
		BIT	SS_PAGE1
		BIT	SS_HIRES_ON
		BIT	SS_FULLSCREEN
		BIT	SS_GRAPHICS
.endmacro

.macro TEXT_INIT
		BIT	SS_PAGE1
		BIT	SS_HIRES_OFF
		BIT	SS_TEXT
.endmacro

; ----- KBD_READ: read+strobe a key into A; sets Z if no key ----------------
;
;   KBD_READ              ; A = key code (bit7 stripped), or 0 if no key.
;
; Returns with Z=1 (BEQ) iff no key was pending.  Strobes KBDSTRB only when
; a key was actually consumed.
; -------------------------------------------------------------------------
.macro KBD_READ
	.local @none, @done
		LDA	KBD
		BPL	@none
		STA	KBDSTRB
		AND	#$7F
		BRA	@done
@none:
		LDA	#0
@done:
.endmacro
