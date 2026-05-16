; ============================================================================
; lib/frame.s -- engine lifecycle.
;
; engine_init        : install jumptable, run lib *_inits, enter HGR.
; engine_frame_begin : input_publish, music tick (TBD), VBL wait (TBD).
; engine_frame_end   : map_draw_dirty, sprite redraw (TBD).
;
; frame_begin/end are installed at $03B3 / $03B6 (above the $0300 ABI
; table, still under the $03BF page-3 cap). Each is a 3-byte JMP, set
; by engine_init via the same JT_SET_SLOT macro that wires the ABI.
; ============================================================================

		.setcpu	"65C02"
		.include	"softswitches.inc"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"input.inc"
		.include	"tilemap.inc"
		.include	"rand.inc"
		.include	"edit.inc"
		.include	"frame.inc"

		.export	engine_init
		.export	engine_frame_begin
		.export	engine_frame_end

		.segment "CODE"

; ----------------------------------------------------------------------------
; engine_init -- bring the engine up. Idempotent.
; Clobbers A, X, Y.
; ----------------------------------------------------------------------------
		.proc	engine_init
		; --- install the $0300 ABI jumptable (all jt_unimpl)
		JSR	jumptable_install
		; --- per-lib init (each patches its own slots)
		JSR	input_init
		JSR	rand_init
		JSR	tilemap_init
		JSR	edit_init
		; --- install lifecycle slots ($03B3 / $03B6)
		; The slots themselves aren't part of the auto-installed
		; template, so we hand-write the JMPs here.
		LDA	#$4C			; JMP opcode
		STA	ENGINE_FRAME_BEGIN
		LDA	#<engine_frame_begin
		STA	ENGINE_FRAME_BEGIN + 1
		LDA	#>engine_frame_begin
		STA	ENGINE_FRAME_BEGIN + 2
		LDA	#$4C
		STA	ENGINE_FRAME_END
		LDA	#<engine_frame_end
		STA	ENGINE_FRAME_END + 1
		LDA	#>engine_frame_end
		STA	ENGINE_FRAME_END + 2
		; --- enter HGR1 full-screen, page 1
		BIT	SS_GRAPHICS	; $C050  graphics mode
		BIT	SS_FULLSCREEN	; $C052  full screen (no mixed text)
		BIT	SS_PAGE1	; $C054  HGR1 displayed
		BIT	SS_HIRES_ON	; $C057  hires (not lores)
		; --- clear HGR1 ($2000-$3FFF)
		LDA	#$00
		LDX	#0
@cl1:		STA	$2000,X
		STA	$2100,X
		STA	$2200,X
		STA	$2300,X
		INX
		BNE	@cl1
		LDX	#0
@cl2:		STA	$2400,X
		STA	$2500,X
		STA	$2600,X
		STA	$2700,X
		INX
		BNE	@cl2
		LDX	#0
@cl3:		STA	$2800,X
		STA	$2900,X
		STA	$2A00,X
		STA	$2B00,X
		INX
		BNE	@cl3
		LDX	#0
@cl4:		STA	$2C00,X
		STA	$2D00,X
		STA	$2E00,X
		STA	$2F00,X
		INX
		BNE	@cl4
		LDX	#0
@cl5:		STA	$3000,X
		STA	$3100,X
		STA	$3200,X
		STA	$3300,X
		INX
		BNE	@cl5
		LDX	#0
@cl6:		STA	$3400,X
		STA	$3500,X
		STA	$3600,X
		STA	$3700,X
		INX
		BNE	@cl6
		LDX	#0
@cl7:		STA	$3800,X
		STA	$3900,X
		STA	$3A00,X
		STA	$3B00,X
		INX
		BNE	@cl7
		LDX	#0
@cl8:		STA	$3C00,X
		STA	$3D00,X
		STA	$3E00,X
		STA	$3F00,X
		INX
		BNE	@cl8
		RTS
		.endproc

; ----------------------------------------------------------------------------
; engine_frame_begin -- per-frame top: refresh input page.
; Music tick + VBL wait land in Phase 5.
; ----------------------------------------------------------------------------
		.proc	engine_frame_begin
		JSR	input_publish
		RTS
		.endproc

; ----------------------------------------------------------------------------
; engine_frame_end -- per-frame bottom: repaint dirty cells.
; Sprite redraw lands in Phase 4.
; ----------------------------------------------------------------------------
		.proc	engine_frame_end
		JSR	map_draw_dirty
		RTS
		.endproc
