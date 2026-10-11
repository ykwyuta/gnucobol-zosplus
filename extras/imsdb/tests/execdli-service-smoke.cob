       >>SOURCE FORMAT FREE
identification division.
program-id. IMSDEXECTEST.
data division.
working-storage section.
01 function-code pic x(4) value 'SCHD'.
01 argument-area pic x(8) value 'IBSCUDAT'.
01 dibstat pic x(2).
procedure division.
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = spaces
        display 'IMS SCHD FIRST FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = 'TC'
        display 'IMS SCHD DUPLICATE FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    move 'TERM' to function-code
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = spaces
        display 'IMS TERM FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    move 'SCHD' to function-code
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = spaces
        display 'IMS SCHD AGAIN FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    move 'CHKP' to function-code
    move 'CHECK001' to argument-area
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = spaces
        display 'IMS CHKP FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    move 'TERM' to function-code
    call 'IMSDEXEC' using function-code argument-area dibstat
    if dibstat not = spaces
        display 'IMS FINAL TERM FAIL: ' dibstat
        move 8 to return-code
        goback
    end-if
    display 'IMS EXEC DLI SERVICES OK'
    goback.
