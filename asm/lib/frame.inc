; ============================================================================
; lib/frame.inc -- engine lifecycle entry points.
;
; Three CALL targets BASIC (or asm) uses each frame:
;
;   engine_init         once at boot, BEFORE first frame.
;                       Installs $0300 ABI jumptable + lifecycle slots,
;                       runs each lib's *_init, sets HGR mode.
;
;   engine_frame_begin  per frame, top of loop.
;                       Polls input -> $0260 page; ticks music; waits VBL.
;
;   engine_frame_end    per frame, bottom of loop.
;                       Repaints dirty tiles; redraws all sprites.
;
; engine_init is at a fixed program address (start of GAME segment).
; frame_begin/end are installed in page 3 at known slots so BASIC can
; CALL them by absolute address even though the engine binary moves.
;
;   $03B3  ENGINE_FRAME_BEGIN   ( BASIC: CALL 947 )
;   $03B6  ENGINE_FRAME_END     ( BASIC: CALL 950 )
;
; engine_init's address is whatever the binary is BLOAD'd at: for the
; DOS 3.3 hires_engine target that's $6000, so BASIC: CALL 24576.
; ============================================================================

	.ifndef _LIB_FRAME_INC_
_LIB_FRAME_INC_	= 1

ENGINE_FRAME_BEGIN	= $03B3
ENGINE_FRAME_END	= $03B6

	.global	engine_init
	.global	engine_frame_begin
	.global	engine_frame_end

	.endif	; _LIB_FRAME_INC_
