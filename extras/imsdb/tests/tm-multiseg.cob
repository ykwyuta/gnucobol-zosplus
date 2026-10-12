       >>SOURCE FORMAT FREE
identification division.
program-id. IMSTMMULTI.
data division.
working-storage section.
01 chng-code pic x(4) value 'CHNG'.
01 isrt-code pic x(4) value 'ISRT'.
01 purg-code pic x(4) value 'PURG'.
01 first-target pic x(8) value 'TERMONE '.
01 second-target pic x(8) value 'TERMTWO '.
01 output-message.
   05 message-length pic 9(4) comp value 8.
   05 message-control pic x(2) value low-values.
   05 message-body pic x(4).
linkage section.
01 io-pcb pic x(32).
01 alt-pcb.
   05 alt-destination pic x(8).
   05 alt-reserved pic x(2).
   05 alt-status pic x(2).
procedure division using io-pcb alt-pcb.
    call 'CBLTDLI' using chng-code alt-pcb first-target
    move 'ONE1' to message-body
    call 'CBLTDLI' using isrt-code alt-pcb output-message
    move 'ONE2' to message-body
    call 'CBLTDLI' using isrt-code alt-pcb output-message
    call 'CBLTDLI' using purg-code alt-pcb
    call 'CBLTDLI' using chng-code alt-pcb second-target
    move 'TWO1' to message-body
    call 'CBLTDLI' using isrt-code alt-pcb output-message
    if alt-status not = spaces
        display 'IMS TM MULTI STATUS ' alt-status
        move 12 to return-code
    else
        move 0 to return-code
    end-if
    goback.
