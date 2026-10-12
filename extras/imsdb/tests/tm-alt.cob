       >>SOURCE FORMAT FREE
identification division.
program-id. IMSTMSMOKE.
data division.
working-storage section.
01 chng-code pic x(4) value 'CHNG'.
01 isrt-code pic x(4) value 'ISRT'.
01 target-name pic x(8) value 'TERMALT '.
01 output-message.
   05 message-length pic 9(4) comp value 8.
   05 message-control pic x(2) value low-values.
   05 message-body pic x(4) value 'ALTO'.
linkage section.
01 io-pcb pic x(32).
01 alt-pcb.
   05 alt-destination pic x(8).
   05 alt-reserved pic x(2).
   05 alt-status pic x(2).
procedure division using io-pcb alt-pcb.
    call 'CBLTDLI' using chng-code alt-pcb target-name
    if alt-status not = spaces
        display 'IMS TM CHNG STATUS ' alt-status
        move 12 to return-code
        goback
    end-if
    call 'CBLTDLI' using isrt-code alt-pcb output-message
    if alt-status not = spaces
        display 'IMS TM ISRT STATUS ' alt-status
        move 12 to return-code
        goback
    end-if
    move 0 to return-code
    goback.
