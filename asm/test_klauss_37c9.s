.setcpu "6502"

.segment "HEADER"
.org $0400  
_start:
    LDA #$42      ; Load A with $42
    LDX #$52      ; Load X with $52
    LDY #$4B      ; Load Y with $4B
    BRK           ; Trigger BRK (software interrupt)

.segment "HANDLER"
.org $9000
INTERRUPT_HANDLER:
    TSX
    LDA $0102,X
    CMP #$30           
    BNE FAIL
    JMP SUCCESS      

.segment "SUCCESS"
.org $9010
SUCCESS:
    NOP
    RTI 

.segment "FAILURE"
.org $9020
FAIL:
    JMP FAIL       

.segment "VECTORS"
.org $FFFC
.word _start
.word INTERRUPT_HANDLER