; ============================================================================
; lib/softswitches.inc -- Apple //e/c I/O soft-switch addresses.
;
; Naming:
;   SS_*       I/O location ($C0xx).  Some are read-toggleable ($C050-$C07F),
;              some are write-only ($C000-$C00F), some are read/write data
;              registers.
;
; Use the `ss_set` / `ss_read` macros from lib/macros.inc for portable access:
;   ss_set SS_HIRES_ON      ; emits BIT or STA depending on the address.
;
; ============================================================================

		; --- keyboard ---
KBD		=	$C000	; r:    last key pressed (bit7 = strobe)
KBDSTRB		=	$C010	; r/w:  clear strobe / read same as KBD

		; --- 80STORE / aux memory ---
SS_80STORE_OFF	=	$C000	; w
SS_80STORE_ON	=	$C001	; w
SS_RAMRD_MAIN	=	$C002	; w
SS_RAMRD_AUX	=	$C003	; w
SS_RAMWR_MAIN	=	$C004	; w
SS_RAMWR_AUX	=	$C005	; w
SS_INTCXROM_OFF	=	$C006	; w
SS_INTCXROM_ON	=	$C007	; w
SS_ALTZP_MAIN	=	$C008	; w
SS_ALTZP_AUX	=	$C009	; w
SS_SLOTC3ROM_OFF =	$C00A	; w
SS_SLOTC3ROM_ON	=	$C00B	; w
SS_80COL_OFF	=	$C00C	; w
SS_80COL_ON	=	$C00D	; w
SS_ALTCHAR_OFF	=	$C00E	; w
SS_ALTCHAR_ON	=	$C00F	; w

		; --- read soft switches (bit 7 = state) ---
RDBNK2		=	$C011
RDLCRAM		=	$C012
RDRAMRD		=	$C013
RDRAMWR		=	$C014
RDCXROM		=	$C015
RDALTZP		=	$C016
RDC3ROM		=	$C017
RDSTORE80	=	$C018
RDVBLBAR	=	$C019	; bit 7 = NOT vbl (0 during vbl)
RDTEXT		=	$C01A
RDMIXED		=	$C01B
RDPAGE2		=	$C01C
RDHIRES		=	$C01D
RDALTCHAR	=	$C01E
RD80COL		=	$C01F

		; --- video soft switches (read-toggle) ---
SS_GRAPHICS	=	$C050
SS_TEXT		=	$C051
SS_FULLSCREEN	=	$C052
SS_MIXED	=	$C053
SS_PAGE1	=	$C054
SS_PAGE2	=	$C055
SS_HIRES_OFF	=	$C056
SS_HIRES_ON	=	$C057

		; --- IOU / annunciators ---
AN0_OFF		=	$C058
AN0_ON		=	$C059
AN1_OFF		=	$C05A
AN1_ON		=	$C05B
AN2_OFF		=	$C05C
AN2_ON		=	$C05D
SS_DHIRES_ON	=	$C05E	; double-hires enable
SS_DHIRES_OFF	=	$C05F

		; --- paddles / buttons ---
OPN_APL		=	$C061	; bit 7 = open-apple (button 0)
SOLID_APL	=	$C062	; bit 7 = solid-apple (button 1)
SHIFT_KEY	=	$C063	; bit 7 = shift on //e+
PADDL0		=	$C064
PADDL1		=	$C065
PADDL2		=	$C066
PADDL3		=	$C067
PTRIG		=	$C070	; trigger paddle one-shot

		; --- speaker ---
SPKR		=	$C030	; r: toggles speaker

		; --- SCC modem (//c) ---
SCC_DATA	=	$C0A8
SCC_STAT	=	$C0A9
