       >>SOURCE FORMAT FREE
identification division.
program-id. IMSBANKVERIFY.
data division.
working-storage section.
01 function-code pic x(4) value 'GU  '.
01 db-pcb.
   05 dbd-name pic x(8) value 'CUSTOMER'.
   05 filler pic x(2).
   05 db-status pic x(2).
   05 filler pic x(44).
01 customer-segment.
   05 customer-id pic x(4).
   05 customer-last pic x(50).
   05 customer-first pic x(50).
   05 filler pic x(175).
01 customer-ssa.
   05 filler pic x(8) value 'CUSTOMER'.
   05 filler pic x value '('.
   05 filler pic x(8) value 'CUSTID  '.
   05 filler pic x(2) value 'EQ'.
   05 searched-id pic s9(9) comp-5 value 1.
   05 filler pic x value ')'.
   05 filler pic x value space.
procedure division.
    call 'CBLTDLI' using function-code db-pcb
        customer-segment customer-ssa
    if db-status not = spaces or customer-first not = 'TESTFIRST'
        display 'IMS BANK REOPEN FAILED: ' db-status
        move 8 to return-code
        goback
    end-if
    display 'IMS BANK REOPEN OK'
    goback.
