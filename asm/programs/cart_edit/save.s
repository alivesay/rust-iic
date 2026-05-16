; ============================================================================
; programs/cart_edit/save.s -- 'V' handler.
;
; Set flag at $0BFF=1, force text mode, and RTS all the way back to BASIC.
; STARTUP (synthesized by mkpodisk --save-loop) sees the flag, BSAVEs CART,
; clears the flag, and re-CALLs us. Crude but zero-asm-MLI.
;
; Stack at entry (handle_key did JMP (ENG_PTR), not JSR):
;   [top] return-to-handle_key-caller (2 bytes from JSR handle_key)
;         return-to-BASIC             (2 bytes from CALL 32768)
; Pop the inner return so RTS exits the editor entirely.
; ============================================================================

		.setcpu	"65C02"

		.export	do_save

		.segment "CODE"

		.proc	do_save
		LDA	#$01
		STA	$0BFF
		LDA	$C051			; force text mode for BASIC
		PLA				; discard handle_key return
		PLA
		RTS				; -> back to BASIC STARTUP
		.endproc
