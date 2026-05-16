; ============================================================================
; lib/maplayout.inc -- runtime RAM layout for tiles, sprite flags, and the
; variable-size tilemap (Wasteland-style: one whole .MAP file per location).
;
;   $4000..$5FFF  TILES file       8 KB raw tile sheet
;   $6000..$60FF  SFLAGS file      256 B per-sprite-id flags
;   $6100..$77FF  MAP file         header (8 B) + L0 + L1 + L2 + MF0 + MF1 + MF2
;   $7800..       editor binary (CARTEDIT)
;
; The editor binary is placed below the BASIC.SYSTEM file-buffer pool
; (which lives at $9D00 - MAXFILES*$400, e.g. $9100-$9CFF for the default
; MAXFILES=3). At base $7800 and a $1A00 size cap, the binary ends at
; $9200 -- safely below the buffer pool no matter what MAXFILES is set to.
;
; .MAP file format (fixed 7936 B = $1F00 on disk regardless of W/H so the
; BSAVE length stays static):
;   $00  W            map width  (1..MAP_MAX_W)
;   $01  H            map height (1..MAP_MAX_H)
;   $02  LAYERS       always 3 for now
;   $03  magic[5]     "RIIC1"
;   $08  L0  data     W*H bytes  (back layer)
;   $08+W*H   L1 data
;   $08+2*W*H L2 data (front)
;   $08+3*W*H MF0 mflags
;   $08+4*W*H MF1
;   $08+5*W*H MF2
;   ...zero pad to $1F00
;
; W*H is bounded by MAP_MAX_CELLS. The engine recomputes layer base
; pointers from W,H whenever the header changes (load, resize).
; ============================================================================

	.ifndef _LIB_MAPLAYOUT_INC_
_LIB_MAPLAYOUT_INC_	= 1

TILES_BASE		= $4000
TILES_BYTES		= $2000		; 8 KB

SPRITE_FLAGS		= $6000
SPRITE_FLAGS_BYTES	= $0100		; 256

; --- map RAM region -------------------------------------------------------
MAP_RAM_BASE		= $6100
MAP_RAM_END		= $7800		; (exclusive) editor binary starts here
MAP_RAM_BYTES		= MAP_RAM_END - MAP_RAM_BASE	; $1700 = 5888

; --- header layout (8 bytes at MAP_RAM_BASE) ------------------------------
MAP_HEADER		= MAP_RAM_BASE
MAP_HDR_W		= MAP_HEADER + 0
MAP_HDR_H		= MAP_HEADER + 1
MAP_HDR_LAYERS		= MAP_HEADER + 2
MAP_HDR_MAGIC		= MAP_HEADER + 3	; 5 bytes
MAP_HDR_BYTES		= 8
MAP_DATA_BASE		= MAP_HEADER + MAP_HDR_BYTES	; $6108
MAP_DATA_BYTES		= MAP_RAM_END - MAP_DATA_BASE	; $16F8 = 5880

; --- layer constants ------------------------------------------------------
MAP_LAYERS		= 3
MAP_BYTES_PER_CELL	= MAP_LAYERS * 2	; 3 data + 3 mflags

; floor(5880 / 6) = 980 cells max (e.g. 31x31, 49x20, 28x35)
MAP_MAX_CELLS		= 980
MAP_MAX_W		= 49
MAP_MAX_H		= 49

MAP_DEFAULT_W		= 20
MAP_DEFAULT_H		= 12

; --- viewport (HGR1 always shows this much) -------------------------------
VIEW_COLS		= 20
VIEW_ROWS		= 12
VIEW_CELLS		= VIEW_COLS * VIEW_ROWS

	.endif	; _LIB_MAPLAYOUT_INC_
