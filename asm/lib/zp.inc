; ============================================================================
; lib/zp.inc -- engine zero-page layout (locked ABI).
;
; See docs/engine2.md "MEMORY MAP" + "ENGINE CALL JUMP TABLE" for rationale.
; Reserved across the engine + lib + IDE; games may use $10-$1F freely.
;
; Layout
;   $00-$05   reserved (DOS / monitor / ProDOS)
;   $06-$0F   ENGINE SCRATCH         engine + lib internal
;   $10-$1F   USER ZP                game free-for-all (engine never touches)
;   $20-$2F   AVAILABLE              (was API args; now at $0270 absolute)
;   $30-$3F   SPRITE SLOTS           4 sprites x 4 bytes (sprite_* only)
;   $40-$4F   MUSIC ZP               music_* / sfx_* only
;   $50-$FF   ca65 runtime / system
; ============================================================================

	.ifndef _LIB_ZP_INC_
_LIB_ZP_INC_	= 1

; ----- engine scratch ($06-$0F) --------------------------------------------
ENG_PTR		= $06	; 16-bit pointer  (lo at $06, hi at $07)
ENG_PTR_HI	= $07
ENG_PTR2	= $08	; 16-bit pointer  (lo at $08, hi at $09)
ENG_PTR2_HI	= $09
ENG_TMP		= $0A
ENG_TMP2	= $0B
ENG_TMP3	= $0C
ENG_TMP4	= $0D
ENG_TMP5	= $0E
ENG_TMP6	= $0F

; ----- API argument slots ($0270-$027F, NOT zero-page) ---------------------
; Caller loads ARGn before JSR through the $0300 jump table.
; Single-byte args; 16-bit args use ARGn (lo) + ARGn+1 (hi).
;
; NOTE: relocated off ZP because $20-$23 collide with Applesoft's text
; window vars (WNDLFT/WNDWDTH/WNDTOP/WNDBTM at $20-$23) and $24-$25 are
; CH/CV. Any BASIC POKE 32..35 to set ARGs would corrupt the text scroll
; window. $0270-$027F sits right after the input page ($0260-$0267) and
; is unused by DOS/Monitor/Applesoft.
;
; BASIC: POKE 624 = ARG0, POKE 625 = ARG1, POKE 626 = ARG2, ...
ARG0		= $0270
ARG1		= $0271
ARG2		= $0272
ARG3		= $0273
ARG4		= $0274
ARG5		= $0275
ARG6		= $0276
ARG7		= $0277
ARG8		= $0278
ARG9		= $0279
ARG10		= $027A
ARG11		= $027B
ARG12		= $027C
ARG13		= $027D
ARG14		= $027E
ARG15		= $027F

; ----- sprite slots ($30-$3F) ----------------------------------------------
; 4 sprites x 4 bytes: { x_byte_col, y_pixel, tile_id, flags }
SPRITE_SLOT_BASE = $30
SPRITE_SLOT_SZ	 = 4
SPRITE_COUNT	 = 4

SPRITE0_X	= $30
SPRITE0_Y	= $31
SPRITE0_TILE	= $32
SPRITE0_FLAGS	= $33

SPRITE1_X	= $34
SPRITE1_Y	= $35
SPRITE1_TILE	= $36
SPRITE1_FLAGS	= $37

SPRITE2_X	= $38
SPRITE2_Y	= $39
SPRITE2_TILE	= $3A
SPRITE2_FLAGS	= $3B

SPRITE3_X	= $3C
SPRITE3_Y	= $3D
SPRITE3_TILE	= $3E
SPRITE3_FLAGS	= $3F

; sprite flag bits
SPF_VISIBLE	= $80
SPF_FLIP_H	= $40
SPF_FLIP_V	= $20
SPF_PRIORITY	= $10

; ----- music ZP ($40-$4F) --------------------------------------------------
MUSIC_ZP_BASE	= $40
MUSIC_ZP_END	= $4F

	.endif	; _LIB_ZP_INC_
