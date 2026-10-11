       >>SOURCE FORMAT FREE
identification division.
program-id. IMSGSAM.
data division.
working-storage section.
01 function-code pic x(4) value 'GN  '.
01 commit-rc pic s9(9) comp-5.
01 gsam-pcb.
   05 dbd-name pic x(8) value 'PASFLDBD'.
   05 segment-level pic x(2).
   05 db-status pic x(2).
   05 process-options pic x(4).
   05 reserved-area pic x(4).
   05 segment-name pic x(8).
   05 key-length pic 9(8) comp.
   05 sensitive-count pic 9(8) comp.
   05 key-feedback pic x(8).
01 record-area pic x(100).
01 rsa pic x(8).
01 first-rsa pic x(8).
procedure division.
    move 'OPEN' to function-code
    call 'CBLTDLI' using function-code gsam-pcb
    if db-status not = spaces
        display 'IMS GSAM OPEN FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move 'GN  ' to function-code
    call 'CBLTDLI' using function-code gsam-pcb record-area rsa
    if db-status not = spaces or record-area(1:5) not = 'FIRST'
        display 'IMS GSAM FIRST FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move rsa to first-rsa
    call 'CBLTDLI' using function-code gsam-pcb record-area rsa
    if db-status not = spaces or record-area(1:6) not = 'SECOND'
        display 'IMS GSAM SECOND FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    call 'CBLTDLI' using function-code gsam-pcb record-area
    if db-status not = 'GB'
        display 'IMS GSAM EOF FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move 'GU  ' to function-code
    move first-rsa to rsa
    call 'CBLTDLI' using function-code gsam-pcb record-area rsa
    if db-status not = spaces or record-area(1:5) not = 'FIRST'
        display 'IMS GSAM GU FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move 'THIRD' to record-area
    move 'ISRT' to function-code
    call 'CBLTDLI' using function-code gsam-pcb record-area rsa
    if db-status not = spaces
        display 'IMS GSAM ISRT FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    call 'IMSDBCOM' returning commit-rc
    if commit-rc not = 0
        display 'IMS GSAM COMMIT FAIL'
        move 1 to return-code
        goback
    end-if
    move 'CLSE' to function-code
    call 'CBLTDLI' using function-code gsam-pcb
    if db-status not = spaces
        display 'IMS GSAM CLSE FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    display 'IMS GSAM OK'
    goback.
