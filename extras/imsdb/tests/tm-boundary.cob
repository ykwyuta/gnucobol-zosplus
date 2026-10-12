       >>SOURCE FORMAT FREE
identification division.
program-id. IMSTMBOUND.
data division.
working-storage section.
01 gu-code pic x(4) value 'GU  '.
01 input-message.
   05 message-length pic 9(4) comp.
   05 message-control pic x(2).
   05 message-body pic x(4).
linkage section.
01 io-pcb.
   05 filler pic x(10).
   05 io-status pic x(2).
   05 filler pic x(20).
procedure division using io-pcb.
    call 'CBLTDLI' using gu-code io-pcb input-message
    if io-status not = spaces or message-body not = 'ONE1'
        move 12 to return-code
        goback
    end-if
    call 'CBLTDLI' using gu-code io-pcb input-message
    if io-status not = spaces or message-body not = 'TWO2'
        move 12 to return-code
        goback
    end-if
    move 8 to return-code
    goback.
