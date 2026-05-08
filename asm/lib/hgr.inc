; ============================================================================
; lib/hgr.inc -- hires graphics helpers.
;
; Apple ][ hires row addressing is famously non-linear:
;
;   addr(y) = $2000
;           + (((y >> 6) & 3) * $28)        ; "third" within the screen (0..2)
;           + (((y >> 3) & 7) * $80)        ; row-of-8 within the third
;           + (((y     ) & 7) * $400)       ; microline within the row-of-8
;
; We precompute two 192-byte tables (`hgr_lo` and `hgr_hi`) once per game.
; Drop the table macro in any segment, then index by Y register.
; ============================================================================

; ----- HGR_ROW_TABLES: emit the 192-entry lookup tables --------------------
; 
;   .align	256
;   HGR_ROW_TABLES        ; emits labels `hgr_lo:` and `hgr_hi:`
;
; The host is responsible for the alignment.  Tables together are 384 bytes;
; aligning to 256 lets later math (e.g. self-mod LDA hgr_lo,X) avoid page
; crossings.
; -------------------------------------------------------------------------
.macro HGR_ROW_TABLES
hgr_lo:
		.repeat	192, ROW
		.byte	(((ROW >> 3) & 1) << 7) + (((ROW >> 6) & 3) * 40)
		.endrep
hgr_hi:
		.repeat	192, ROW
		.byte	$20 + ((ROW & 7) << 2) + (((ROW >> 3) & 7) >> 1)
		.endrep
.endmacro

; ----- hgr_row_addr: compute the address of (col=0, row=Y_REG) into ptr ----
;
;   hgr_row_addr ptr, row_x         ; ptr <- HGR_BASE + offset(row_x)
;
; Inputs:
;   row_x  -- a register that already holds the row index (only X supported
;             today; Y indexes our own tables).
;   ptr    -- destination zero-page lo address.
;
; Clobbers A.  Preserves Y.  Requires `hgr_lo` / `hgr_hi` tables present.
; -------------------------------------------------------------------------
.macro hgr_row_addr ptr, row_x
		LDA	hgr_lo,row_x
		STA	ptr
		LDA	hgr_hi,row_x
		STA	ptr+1
.endmacro

; ----- hgr_cell_addr: address of (col=col_byte, row=row_x) into ptr -------
;
;   hgr_cell_addr ptr, row_x, col_byte
;
; col_byte is an 8-bit value (0..39).  Clobbers A.  Preserves X, Y.
; -------------------------------------------------------------------------
.macro hgr_cell_addr ptr, row_x, col_byte
		LDA	hgr_lo,row_x
		CLC
		ADC	col_byte
		STA	ptr
		LDA	hgr_hi,row_x
		ADC	#0
		STA	ptr+1
.endmacro
