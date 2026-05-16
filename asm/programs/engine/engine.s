; ============================================================================
; programs/engine/engine.s -- shared engine binary for BASIC-driven games.
;
; This is the only asm any BASIC game needs. It's BLOAD'd at $8000;
; BASIC line 2 does CALL 32768 to land on engine_init.
;
; Once we move to ProDOS this becomes ENGINE.SYSTEM and lives in /HIRES/.
; ============================================================================

		.setcpu	"65C02"

		.import	engine_init

		.segment "GAME"

; ----- $8000: BASIC entry point (CALL 32768) --------------------------------
		JMP	engine_init
