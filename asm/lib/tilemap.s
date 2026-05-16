; ============================================================================
; lib/tilemap.s -- variable-size tilemap with 20x12 viewport.
;
; Map storage layout: see lib/maplayout.inc. Header at $6100, then six
; W*H byte planes (3 data + 3 mflags) packed contiguously. Layer base
; pointers live in BSS and are recomputed whenever W or H change.
;
; All public APIs that take map coordinates use WORLD coords. The engine
; tracks (cam_x, cam_y); the viewport shows the rectangle
; [cam_x..cam_x+VIEW_COLS) x [cam_y..cam_y+VIEW_ROWS).
;
; HGR row tables (`hgr_lo` / `hgr_hi`) are imported from the host program
; (still emitted by hires_engine.s for now).
; ============================================================================

		.setcpu	"65C02"
		.include	"zp.inc"
		.include	"jumptable.inc"
		.include	"tilemap.inc"

		.import	hgr_lo
		.import	hgr_hi

		.export	tilemap_init
		.export	map_init
		.export	map_recompute_ptrs
		.export	map_set_cam
		.export	map_resize
		.export	tile_draw
		.export	tile_set_map
		.export	map_draw_all
		.export	map_draw_dirty
		.export	tile_set_sheet
		.export	sheet_ptr_lo
		.export	sheet_ptr_hi

		; --- runtime state (read by editor) ---
		.export	map_w
		.export	map_h
		.export	cam_x
		.export	cam_y
		.export	map_l0_lo, map_l0_hi
		.export	map_l1_lo, map_l1_hi
		.export	map_l2_lo, map_l2_hi
		.export	map_mf0_lo, map_mf0_hi
		.export	map_mf1_lo, map_mf1_hi
		.export	map_mf2_lo, map_mf2_hi

; ZP scratch (engine pool $06-$0F). Documented use:
;   ENG_PTR  ($06/$07)  HGR / map destination pointer
;   ENG_PTR2 ($08/$09)  tile-data source pointer
;   ENG_TMP  ($0A)      scratch
;   ENG_TMP2 ($0B)      col_byte / scratch
;   ENG_TMP3 ($0C)      pixel_row base / scratch
;   ENG_TMP4 ($0D)      visible cell idx (0..239)
;   ENG_TMP5 ($0E)      idx16 hi (mul accumulator high byte)
;   ENG_TMP6 ($0F)      idx16 lo / multiplier scratch

; ----- BSS ------------------------------------------------------------------
		.segment "BSS"
sheet_ptr_lo:	.res	1
sheet_ptr_hi:	.res	1

map_w:		.res	1
map_h:		.res	1
cam_x:		.res	1
cam_y:		.res	1

map_l0_lo:	.res	1
map_l0_hi:	.res	1
map_l1_lo:	.res	1
map_l1_hi:	.res	1
map_l2_lo:	.res	1
map_l2_hi:	.res	1
map_mf0_lo:	.res	1
map_mf0_hi:	.res	1
map_mf1_lo:	.res	1
map_mf1_hi:	.res	1
map_mf2_lo:	.res	1
map_mf2_hi:	.res	1

idx_lo:		.res	1	; 16-bit cell index scratch
idx_hi:		.res	1
mul_a:		.res	1
mul_b:		.res	1

set_layer:	.res	1
set_tile:	.res	1
set_wcol:	.res	1
set_wrow:	.res	1

dd_vc:		.res	1	; map_draw_dirty: visible col
dd_vr:		.res	1	; visible row
dd_wcol:	.res	1	; world col = vc + cam_x
dd_wrow:	.res	1	; world row = vr + cam_y

; row pointers cached during map_draw_all (one mul per row instead of one
; per cell). row_ptr_lN points at world cell (cam_x, wrow) on layer N.
row_ptr_l0:	.res	2
row_ptr_l1:	.res	2
row_ptr_l2:	.res	2
row_blank_n:	.res	1	; blank-fill counter (cols outside map_w)
cell_t0:	.res	1	; tile id at current cell, layer 0
cell_t1:	.res	1
cell_t2:	.res	1

		.segment "CODE"

; ----------------------------------------------------------------------------
; tilemap_init -- wire ABI slots, install default sheet ptr, validate map
; header. Safe to call before any TILES/MAP files have been BLOAD'd: bad
; magic triggers a default 20x12 init.
; ----------------------------------------------------------------------------
		.proc	tilemap_init
		LDA	#0
		LDX	#30
@cd:		STA	TILEMAP_DIRTY - 1,X
		DEX
		BNE	@cd
		LDX	#<TILES_BASE
		LDY	#>TILES_BASE
		JSR	tile_set_sheet
		JT_SET_SLOT JT_TILE_DRAW,      jt_tile_draw
		JT_SET_SLOT JT_TILE_SET_MAP,   jt_tile_set_map
		JT_SET_SLOT JT_MAP_DRAW_ALL,   jt_map_draw_all
		JT_SET_SLOT JT_MAP_DRAW_DIRTY, jt_map_draw_dirty
		JT_SET_SLOT JT_TILE_AT,        jt_tile_at
		JT_SET_SLOT JT_TILE_SET_MAP_L, jt_tile_set_map_l
		JT_SET_SLOT JT_TILE_AT_L,      jt_tile_at_l
		JT_SET_SLOT JT_MAP_SET_CAM,    jt_map_set_cam
		JT_SET_SLOT JT_MAP_RESIZE,     jt_map_resize
		STZ	cam_x
		STZ	cam_y
		JSR	map_init
		RTS
		.endproc

; ----------------------------------------------------------------------------
; tile_set_sheet -- X=lo, Y=hi: ptr to a 256-tile sheet (8 KB).
; ----------------------------------------------------------------------------
		.proc	tile_set_sheet
		STX	sheet_ptr_lo
		STY	sheet_ptr_hi
		RTS
		.endproc

; ============================================================================
; map_init -- validate the header at $6100. Magic mismatch (or W==0,
; H==0, oversize) triggers a fresh default-size init with zeroed data.
; Always recomputes layer pointers and resets cam to (0,0).
; ============================================================================
		.proc	map_init
		LDA	MAP_HDR_MAGIC + 0
		CMP	#'R'
		BNE	@reinit
		LDA	MAP_HDR_MAGIC + 1
		CMP	#'I'
		BNE	@reinit
		LDA	MAP_HDR_MAGIC + 2
		CMP	#'I'
		BNE	@reinit
		LDA	MAP_HDR_MAGIC + 3
		CMP	#'C'
		BNE	@reinit
		LDA	MAP_HDR_MAGIC + 4
		CMP	#'1'
		BNE	@reinit
		; magic OK; sanity check W/H
		LDA	MAP_HDR_W
		BEQ	@reinit
		CMP	#MAP_MAX_W + 1
		BCS	@reinit
		LDA	MAP_HDR_H
		BEQ	@reinit
		CMP	#MAP_MAX_H + 1
		BCS	@reinit
		; W*H <= MAP_MAX_CELLS?
		LDA	MAP_HDR_W
		LDX	MAP_HDR_H
		JSR	mul8x8			; result -> idx_lo/idx_hi
		LDA	idx_hi
		CMP	#>(MAP_MAX_CELLS + 1)
		BCC	@accept
		BNE	@reinit
		LDA	idx_lo
		CMP	#<(MAP_MAX_CELLS + 1)
		BCS	@reinit
@accept:
		LDA	MAP_HDR_W
		STA	map_w
		LDA	MAP_HDR_H
		STA	map_h
		BRA	@finish

@reinit:
		LDA	#MAP_DEFAULT_W
		STA	MAP_HDR_W
		STA	map_w
		LDA	#MAP_DEFAULT_H
		STA	MAP_HDR_H
		STA	map_h
		LDA	#MAP_LAYERS
		STA	MAP_HDR_LAYERS
		LDA	#'R'
		STA	MAP_HDR_MAGIC + 0
		LDA	#'I'
		STA	MAP_HDR_MAGIC + 1
		STA	MAP_HDR_MAGIC + 2
		LDA	#'C'
		STA	MAP_HDR_MAGIC + 3
		LDA	#'1'
		STA	MAP_HDR_MAGIC + 4
		; zero the entire map data area
		JSR	zero_map_data

@finish:
		STZ	cam_x
		STZ	cam_y
		JSR	map_recompute_ptrs
		RTS
		.endproc

; ============================================================================
; zero_map_data -- write $00 to MAP_DATA_BASE..MAP_RAM_END-1.
; Clobbers A, X, Y, ENG_PTR.
;
; The first byte we want to zero is at MAP_DATA_BASE = $6108. The inner
; "fill 256 bytes via Y" trick only works when ENG_PTR is page-aligned,
; otherwise the final page write crosses MAP_RAM_END and clobbers code
; living right after it. We sidestep that by:
;   1. Zero the partial first page ($6108..$61FF) with an explicit Y loop.
;   2. Page-align ENG_PTR to $6200 and zero whole pages up to MAP_RAM_END.
; ============================================================================
		.proc	zero_map_data
		; --- step 1: zero the partial page MAP_DATA_BASE..(page_end-1)
		LDA	#>MAP_DATA_BASE
		STA	ENG_PTR + 1
		LDA	#0
		STA	ENG_PTR			; ENG_PTR = $XX00
		LDY	#<MAP_DATA_BASE		; start offset within the page
		LDA	#0
@p1:		STA	(ENG_PTR),Y
		INY
		BNE	@p1
		; --- step 2: zero whole pages up to MAP_RAM_END
		INC	ENG_PTR + 1		; advance to next page
@page:
		LDA	ENG_PTR + 1
		CMP	#>MAP_RAM_END
		BCS	@done
		LDY	#0
		LDA	#0
@p2:		STA	(ENG_PTR),Y
		INY
		BNE	@p2
		INC	ENG_PTR + 1
		BRA	@page
@done:
		RTS
		.endproc

; ============================================================================
; map_recompute_ptrs -- given map_w, map_h, set the six layer base ptrs.
; planeN = MAP_DATA_BASE + N * (W*H).
; ============================================================================
		.proc	map_recompute_ptrs
		LDA	map_w
		LDX	map_h
		JSR	mul8x8			; idx_lo/idx_hi = W*H

		; --- L0 = MAP_DATA_BASE
		LDA	#<MAP_DATA_BASE
		STA	map_l0_lo
		LDA	#>MAP_DATA_BASE
		STA	map_l0_hi

		; --- L1 = L0 + WH
		CLC
		LDA	map_l0_lo
		ADC	idx_lo
		STA	map_l1_lo
		LDA	map_l0_hi
		ADC	idx_hi
		STA	map_l1_hi

		; --- L2 = L1 + WH
		CLC
		LDA	map_l1_lo
		ADC	idx_lo
		STA	map_l2_lo
		LDA	map_l1_hi
		ADC	idx_hi
		STA	map_l2_hi

		; --- MF0 = L2 + WH
		CLC
		LDA	map_l2_lo
		ADC	idx_lo
		STA	map_mf0_lo
		LDA	map_l2_hi
		ADC	idx_hi
		STA	map_mf0_hi

		; --- MF1 = MF0 + WH
		CLC
		LDA	map_mf0_lo
		ADC	idx_lo
		STA	map_mf1_lo
		LDA	map_mf0_hi
		ADC	idx_hi
		STA	map_mf1_hi

		; --- MF2 = MF1 + WH
		CLC
		LDA	map_mf1_lo
		ADC	idx_lo
		STA	map_mf2_lo
		LDA	map_mf1_hi
		ADC	idx_hi
		STA	map_mf2_hi
		RTS
		.endproc

; ============================================================================
; mul8x8 -- A * X -> 16-bit result in (idx_lo, idx_hi).
; Standard shift-and-add: high accum in A, low accum in idx_lo, shift
; right after each conditional add to align next bit. Clobbers A, X, mul_a,
; mul_b. Result fits in 16 bits since W,H <= 64 -> max product 4096.
; ============================================================================
		.proc	mul8x8
		STA	mul_a
		STX	mul_b
		LDA	#0
		STA	idx_lo
		LDX	#8
@l:
		LSR	mul_b		; carry = next multiplier bit
		BCC	@nadd
		CLC
		ADC	mul_a
@nadd:
		ROR			; ROR A: shift in carry, bit0 out
		ROR	idx_lo
		DEX
		BNE	@l
		STA	idx_hi
		RTS
		.endproc

; ============================================================================
; ptr_for_layer_y -- A=layer (0..2), output ENG_PTR2 = data layer base
; (lo,hi) for that layer. Clobbers A,Y. Used for data planes (L0/L1/L2).
; ============================================================================
		.proc	ptr_for_layer
		ASL				; layer * 2
		TAY
		LDA	layer_lo_tbl,Y
		STA	ENG_PTR2
		LDA	layer_lo_tbl + 1,Y
		STA	ENG_PTR2 + 1
		RTS
		.endproc

; In-RAM "table" of layer base ZP addresses isn't possible (the bases live
; in BSS). We index a code-table of indirect loads instead.
layer_lo_tbl:
		.addr	map_l0_lo
		.addr	map_l1_lo
		.addr	map_l2_lo

; ============================================================================
; cell_index -- compute idx16 = wrow*MAP_W + wcol into (idx_lo, idx_hi).
; Inputs:  wrow in A, wcol in X.
; Clobbers A, X, Y, mul_a/b/idx_lo/idx_hi.
; ============================================================================
		.proc	cell_index
		PHX			; save wcol
		LDX	map_w
		JSR	mul8x8		; idx = wrow * W
		PLA			; wcol
		CLC
		ADC	idx_lo
		STA	idx_lo
		LDA	idx_hi
		ADC	#0
		STA	idx_hi
		RTS
		.endproc

; ============================================================================
; layer_cell_addr -- ENG_PTR := layer_base[layer] + idx16 (idx already
; computed in idx_lo/idx_hi). A=layer (0..2). Layer >2 silently uses 0.
; Clobbers A, Y, ENG_PTR.
; ============================================================================
		.proc	layer_cell_addr
		CMP	#MAP_LAYERS
		BCC	:+
		LDA	#0
:		ASL			; layer * 2
		TAY
		CLC
		LDA	idx_lo
		ADC	layer_addr_tbl,Y
		STA	ENG_PTR
		LDA	idx_hi
		ADC	layer_addr_tbl + 1,Y
		STA	ENG_PTR + 1
		RTS
		.endproc

; The ZP table holds the *current values* of map_l{0,1,2}_lo/hi, refreshed
; before each lookup. Reading map_l0_lo directly here would require
; address-known-at-assembly indexing; we instead bounce through this code
; that LDAs each base ptr explicitly per layer. Cheaper: a small dispatch.
		.proc	get_layer_base
		; A = layer, output: A=lo, X=hi
		CMP	#1
		BEQ	@l1
		CMP	#2
		BEQ	@l2
		LDA	map_l0_lo
		LDX	map_l0_hi
		RTS
@l1:		LDA	map_l1_lo
		LDX	map_l1_hi
		RTS
@l2:		LDA	map_l2_lo
		LDX	map_l2_hi
		RTS
		.endproc

; layer_addr_tbl is unused but kept as documentation of intent; replaced
; by the get_layer_base dispatcher above. Provided as a small data table
; in case future code wants to index by layer.
layer_addr_tbl:
		.byte	0,0,0,0,0,0

; ============================================================================
; cell_addr_for -- compute ENG_PTR = data layer base + idx16 for layer in
; A. Idx must already be in (idx_lo, idx_hi). Clobbers A, X, Y, ENG_PTR.
; ============================================================================
		.proc	cell_addr_for
		JSR	get_layer_base	; A=lo, X=hi
		CLC
		ADC	idx_lo
		STA	ENG_PTR
		TXA
		ADC	idx_hi
		STA	ENG_PTR + 1
		RTS
		.endproc

; ============================================================================
; mflag_addr_for -- compute ENG_PTR = mflag layer base + idx16 for layer
; in A. Idx must already be in (idx_lo, idx_hi). Clobbers A, X, Y, ENG_PTR.
; ============================================================================
		.proc	mflag_addr_for
		CMP	#1
		BEQ	@l1
		CMP	#2
		BEQ	@l2
		LDA	map_mf0_lo
		LDX	map_mf0_hi
		BRA	@add
@l1:		LDA	map_mf1_lo
		LDX	map_mf1_hi
		BRA	@add
@l2:		LDA	map_mf2_lo
		LDX	map_mf2_hi
@add:		CLC
		ADC	idx_lo
		STA	ENG_PTR
		TXA
		ADC	idx_hi
		STA	ENG_PTR + 1
		RTS
		.endproc

; ============================================================================
; tile_set_map -- layer-0 alias.
; In: ARG0=tile, ARG1=wcol, ARG2=wrow.
; ============================================================================
		.proc	tile_set_map
		LDA	#0
		STA	set_layer
		JMP	tile_set_map_inner
		.endproc

; ============================================================================
; jt_tile_set_map_l -- ARG0=tile, ARG1=wcol, ARG2=wrow, ARG3=layer.
; ============================================================================
		.proc	jt_tile_set_map_l
		LDA	ARG3
		CMP	#MAP_LAYERS
		BCC	:+
		LDA	#0
:		STA	set_layer
		JSR	tile_set_map_inner
		CLC
		RTS
		.endproc

; ============================================================================
; tile_set_map_inner -- shared core.
; Reads ARG0 (tile), ARG1 (wcol), ARG2 (wrow), set_layer.
; Writes map[layer][wrow*MAP_W + wcol] = tile, marks viewport cell dirty
; if (wcol,wrow) is currently visible.
; ============================================================================
		.proc	tile_set_map_inner
		; clamp wcol < W, wrow < H -> silently no-op out of range
		LDA	ARG1
		CMP	map_w
		BCS	@out
		LDA	ARG2
		CMP	map_h
		BCS	@out

		; idx = wrow * W + wcol
		LDA	ARG2
		LDX	ARG1
		JSR	cell_index

		LDA	set_layer
		JSR	cell_addr_for	; ENG_PTR -> map[layer][idx]

		LDA	ARG0
		LDY	#0
		STA	(ENG_PTR),Y

		; --- mark viewport cell dirty if visible
		; vc = wcol - cam_x; if borrow or >= VIEW_COLS skip
		SEC
		LDA	ARG1
		SBC	cam_x
		BCC	@out
		CMP	#VIEW_COLS
		BCS	@out
		STA	ENG_TMP2	; vc
		SEC
		LDA	ARG2
		SBC	cam_y
		BCC	@out
		CMP	#VIEW_ROWS
		BCS	@out
		; visible -> mark dirty (vidx = vr*20 + vc)
		; vr*20: A *= 20 via lookup-free shift+add (small, only 0..11)
		STA	ENG_TMP		; vr
		ASL			; *2
		ASL			; *4
		STA	ENG_TMP3	; *4
		ASL			; *8
		ASL			; *16
		CLC
		ADC	ENG_TMP3	; *20
		CLC
		ADC	ENG_TMP2	; +vc
		TAX
		TXA
		LSR
		LSR
		LSR
		TAY			; byte 0..29
		TXA
		AND	#$07
		TAX			; bit 0..7
		LDA	bitmask_lut,X
		ORA	TILEMAP_DIRTY,Y
		STA	TILEMAP_DIRTY,Y
@out:
		RTS
		.endproc

; ============================================================================
; jt_tile_at / jt_tile_at_l -- read map[layer][wrow*MAP_W + wcol] -> ARG0.
; Out-of-range (wcol>=W or wrow>=H or layer>=3) returns 0.
; ============================================================================
		.proc	jt_tile_at
		LDA	#0
		STA	set_layer
		JMP	tile_at_inner
		.endproc

		.proc	jt_tile_at_l
		LDA	ARG2
		CMP	#MAP_LAYERS
		BCC	:+
		LDA	#0
:		STA	set_layer
		; fall through
		.endproc

		.proc	tile_at_inner
		LDA	ARG0
		CMP	map_w
		BCS	@oob
		LDA	ARG1
		CMP	map_h
		BCS	@oob
		LDA	ARG1
		LDX	ARG0
		JSR	cell_index
		LDA	set_layer
		JSR	cell_addr_for
		LDY	#0
		LDA	(ENG_PTR),Y
		STA	ARG0
		CLC
		RTS
@oob:
		LDA	#0
		STA	ARG0
		CLC
		RTS
		.endproc

; ============================================================================
; map_set_cam -- A=cam_x, X=cam_y. Mark every viewport cell dirty so the
; next map_draw_dirty repaints the whole window.
; ABI wrapper: jt_map_set_cam reads ARG0,ARG1.
; ============================================================================
		.proc	map_set_cam
		STA	cam_x
		STX	cam_y
		LDA	#$FF
		LDX	#29
@m:		STA	TILEMAP_DIRTY,X
		DEX
		BPL	@m
		RTS
		.endproc

		.proc	jt_map_set_cam
		LDA	ARG0
		LDX	ARG1
		JSR	map_set_cam
		CLC
		RTS
		.endproc

; ============================================================================
; map_resize -- A=W, X=H. Rejects W==0, H==0, or W*H>MAP_MAX_CELLS.
; On accept: writes header, zeros all data planes, recomputes ptrs, marks
; viewport dirty. Caller does the redraw.
; Returns: carry CLEAR on success, carry SET on bad size.
; ============================================================================
		.proc	map_resize
		CMP	#0
		BEQ	@bad
		CPX	#0
		BEQ	@bad
		CMP	#MAP_MAX_W + 1
		BCS	@bad
		PHA
		TXA
		CMP	#MAP_MAX_H + 1
		BCS	@bad_pull
		PLA
		PHA
		PHX
		JSR	mul8x8
		LDA	idx_hi
		CMP	#>(MAP_MAX_CELLS + 1)
		BCC	@accept
		BNE	@bad_pull2
		LDA	idx_lo
		CMP	#<(MAP_MAX_CELLS + 1)
		BCS	@bad_pull2
@accept:
		PLX			; H
		PLA			; W
		STA	map_w
		STA	MAP_HDR_W
		STX	map_h
		STX	MAP_HDR_H
		LDA	#MAP_LAYERS
		STA	MAP_HDR_LAYERS
		JSR	zero_map_data
		JSR	map_recompute_ptrs
		LDA	#$FF
		LDX	#29
@md:		STA	TILEMAP_DIRTY,X
		DEX
		BPL	@md
		CLC
		RTS
@bad_pull2:
		PLX
@bad_pull:
		PLA
@bad:
		SEC
		RTS
		.endproc

		.proc	jt_map_resize
		LDA	ARG0
		LDX	ARG1
		JSR	map_resize
		RTS
		.endproc

; ============================================================================
; map_draw_all -- repaint the entire viewport in row-major order.
;
; Fast path: one mul8x8 per row (12 muls) instead of one per cell (240),
; with row layer-pointers cached and column stepping done by Y indexing.
; Skips the dirty bitmap entirely (and clears it at the end since every
; cell is freshly painted).
;
; Layout per row:
;   wrow       = vr + cam_y
;   row_base   = wrow * map_w + cam_x  (16-bit)
;   row_ptr_lN = map_lN + row_base     (one ptr per layer)
;   For vc in 0..VIEW_COLS-1:
;     wcol = vc + cam_x
;     if wcol >= map_w: blank fill remainder of row
;     else: tile = (row_ptr_lN),Y; if non-zero (or L0) draw at (vc, vr)
; ============================================================================
		.proc	map_draw_all
		LDA	#0
		STA	dd_vr
@row_loop:
		; wrow = vr + cam_y
		LDA	dd_vr
		CLC
		ADC	cam_y
		STA	dd_wrow
		CMP	map_h
		BCC	@in_row
		; entire row out-of-bounds: blank fill
		LDA	#VIEW_COLS
		STA	row_blank_n
		LDA	#0
		STA	dd_vc
		JMP	@blank_fill
@in_row:
		; idx = wrow * map_w
		LDA	dd_wrow
		LDX	map_w
		JSR	mul8x8			; -> idx_lo, idx_hi
		; idx += cam_x
		LDA	cam_x
		CLC
		ADC	idx_lo
		STA	idx_lo
		LDA	idx_hi
		ADC	#0
		STA	idx_hi
		; row_ptr_l0 = map_l0 + idx
		CLC
		LDA	map_l0_lo
		ADC	idx_lo
		STA	row_ptr_l0
		LDA	map_l0_hi
		ADC	idx_hi
		STA	row_ptr_l0 + 1
		CLC
		LDA	map_l1_lo
		ADC	idx_lo
		STA	row_ptr_l1
		LDA	map_l1_hi
		ADC	idx_hi
		STA	row_ptr_l1 + 1
		CLC
		LDA	map_l2_lo
		ADC	idx_lo
		STA	row_ptr_l2
		LDA	map_l2_hi
		ADC	idx_hi
		STA	row_ptr_l2 + 1
		; col loop
		LDA	#0
		STA	dd_vc
@col_loop:
		; wcol = vc + cam_x; check < map_w
		LDA	dd_vc
		CLC
		ADC	cam_x
		CMP	map_w
		BCC	:+
		JMP	@blank_rest
:
		; Read tile from each layer into scratch BEFORE any tile_draw
		; (tile_draw clobbers ENG_PTR/ENG_PTR2). Indirect-Y needs the
		; pointer in ZP, so load each row_ptr_lN into ENG_PTR first.
		LDY	dd_vc
		LDA	row_ptr_l0
		STA	ENG_PTR
		LDA	row_ptr_l0 + 1
		STA	ENG_PTR + 1
		LDA	(ENG_PTR),Y
		STA	cell_t0
		LDA	row_ptr_l1
		STA	ENG_PTR
		LDA	row_ptr_l1 + 1
		STA	ENG_PTR + 1
		LDA	(ENG_PTR),Y
		STA	cell_t1
		LDA	row_ptr_l2
		STA	ENG_PTR
		LDA	row_ptr_l2 + 1
		STA	ENG_PTR + 1
		LDA	(ENG_PTR),Y
		STA	cell_t2
		; --- L0 always
		LDA	cell_t0
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
		; --- L1 if non-zero
		LDA	cell_t1
		BEQ	@l2
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
@l2:
		LDA	cell_t2
		BEQ	@col_next
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
@col_next:
		INC	dd_vc
		LDA	dd_vc
		CMP	#VIEW_COLS
		BEQ	:+
		JMP	@col_loop
:		JMP	@row_next
@blank_rest:
		; from current dd_vc to VIEW_COLS-1, paint blank tile (0)
		LDA	#VIEW_COLS
		SEC
		SBC	dd_vc
		STA	row_blank_n
@blank_fill:
		LDA	row_blank_n
		BEQ	@row_next
		LDA	#0
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
		INC	dd_vc
		DEC	row_blank_n
		BRA	@blank_fill
@row_next:
		INC	dd_vr
		LDA	dd_vr
		CMP	#VIEW_ROWS
		BEQ	@done
		JMP	@row_loop
@done:
		; viewport fully painted -> clear all dirty bits
		LDA	#0
		LDX	#29
@cd:		STA	TILEMAP_DIRTY,X
		DEX
		BPL	@cd
		RTS
		.endproc

		.proc	jt_map_draw_all
		JSR	map_draw_all
		CLC
		RTS
		.endproc

; ============================================================================
; map_draw_dirty -- repaint dirty viewport cells.
; For each visible (vc, vr) with dirty bit set:
;   wcol = vc + cam_x;  wrow = vr + cam_y
;   if (wcol >= MAP_W) or (wrow >= MAP_H): paint blank tile (0)
;   else: paint L0; if L1!=0 paint L1; if L2!=0 paint L2
; ============================================================================
		.proc	map_draw_dirty
		LDA	#0
		STA	ENG_TMP4		; vidx
@loop:
		LDX	ENG_TMP4
		TXA
		LSR
		LSR
		LSR
		TAY				; byte 0..29
		TXA
		AND	#$07
		TAX				; bit
		LDA	bitmask_lut,X
		AND	TILEMAP_DIRTY,Y
		BNE	@hit
		JMP	@next
@hit:
		; clear bit
		LDA	bitmask_lut,X
		EOR	#$FF
		AND	TILEMAP_DIRTY,Y
		STA	TILEMAP_DIRTY,Y
		; decode (vc, vr) from vidx
		JSR	vidx_decode		; ARG1=vc, ARG2=vr, dd_vc/vr set
		; world coords
		CLC
		LDA	dd_vc
		ADC	cam_x
		STA	dd_wcol
		CLC
		LDA	dd_vr
		ADC	cam_y
		STA	dd_wrow
		; out-of-bounds -> blank
		LDA	dd_wcol
		CMP	map_w
		BCS	@blank
		LDA	dd_wrow
		CMP	map_h
		BCS	@blank
		; idx16 = wrow * W + wcol
		LDA	dd_wrow
		LDX	dd_wcol
		JSR	cell_index
		; --- L0 always paint
		LDA	#0
		JSR	cell_addr_for
		LDY	#0
		LDA	(ENG_PTR),Y
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
		; --- L1 if non-zero
		LDA	#1
		JSR	cell_addr_for
		LDY	#0
		LDA	(ENG_PTR),Y
		BEQ	@l2
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
@l2:
		LDA	#2
		JSR	cell_addr_for
		LDY	#0
		LDA	(ENG_PTR),Y
		BEQ	@next
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
		BRA	@next
@blank:
		LDA	#0
		STA	ARG0
		LDA	dd_vc
		STA	ARG1
		LDA	dd_vr
		STA	ARG2
		JSR	tile_draw
@next:
		INC	ENG_TMP4
		LDA	ENG_TMP4
		CMP	#VIEW_CELLS
		BEQ	@done
		JMP	@loop
@done:
		RTS
		.endproc

		.proc	jt_map_draw_dirty
		JSR	map_draw_dirty
		CLC
		RTS
		.endproc

; ============================================================================
; vidx_decode -- ENG_TMP4 = vidx (0..239) -> dd_vc, dd_vr (also ARG1,ARG2).
; vc = vidx mod 20; vr = vidx / 20.
; ============================================================================
		.proc	vidx_decode
		LDA	ENG_TMP4
		LDX	#0
@l:		CMP	#VIEW_COLS
		BCC	@done
		SEC
		SBC	#VIEW_COLS
		INX
		BRA	@l
@done:
		STA	dd_vc
		STA	ARG1
		STX	dd_vr
		STX	ARG2
		RTS
		.endproc

; ============================================================================
; tile_draw -- blit one 14x16 tile to HGR1 viewport cell.
; In:  ARG0=tile, ARG1=col (0..19), ARG2=row (0..11).
; Pure HGR write; never touches map storage.
; ============================================================================
		.proc	tile_draw
		LDA	ARG0
		STA	ENG_PTR2
		LDA	#0
		STA	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		ASL	ENG_PTR2
		ROL	ENG_PTR2 + 1
		CLC
		LDA	ENG_PTR2
		ADC	sheet_ptr_lo
		STA	ENG_PTR2
		LDA	ENG_PTR2 + 1
		ADC	sheet_ptr_hi
		STA	ENG_PTR2 + 1

		LDA	ARG1
		ASL
		STA	ENG_TMP2

		LDA	ARG2
		ASL
		ASL
		ASL
		ASL
		STA	ENG_TMP3

		LDA	#0
		STA	ENG_TMP
@row_loop:
		LDA	ENG_TMP3
		CLC
		ADC	ENG_TMP
		TAX
		LDA	hgr_lo,X
		CLC
		ADC	ENG_TMP2
		STA	ENG_PTR
		LDA	hgr_hi,X
		ADC	#0
		STA	ENG_PTR + 1

		LDA	ENG_TMP
		ASL
		TAY
		LDA	(ENG_PTR2),Y
		LDY	#0
		STA	(ENG_PTR),Y

		LDA	ENG_TMP
		ASL
		ORA	#$01
		TAY
		LDA	(ENG_PTR2),Y
		LDY	#1
		STA	(ENG_PTR),Y

		INC	ENG_TMP
		LDA	ENG_TMP
		CMP	#16
		BNE	@row_loop
		RTS
		.endproc

; ============================================================================
; jt_tile_draw -- ABI thunk.
; ============================================================================
		.proc	jt_tile_draw
		JSR	tile_draw
		CLC
		RTS
		.endproc

		.proc	jt_tile_set_map
		JSR	tile_set_map
		CLC
		RTS
		.endproc

bitmask_lut:	.byte	$01,$02,$04,$08,$10,$20,$40,$80
