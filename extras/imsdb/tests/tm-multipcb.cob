       >>SOURCE FORMAT FREE
identification division.
program-id. IMSTMPCBS.
data division.
linkage section.
01 io-pcb pic x(32).
01 first-pcb.
   05 first-dbd pic x(8).
   05 filler pic x(56).
01 second-pcb.
   05 second-dbd pic x(8).
   05 filler pic x(56).
procedure division using io-pcb first-pcb second-pcb.
    if first-dbd not = 'ACCOUNT ' or second-dbd not = 'HISTORY '
        display 'IMS PCB ORDER WRONG: ' first-dbd ' / ' second-dbd
        move 12 to return-code
    else
        move 0 to return-code
    end-if
    goback.
