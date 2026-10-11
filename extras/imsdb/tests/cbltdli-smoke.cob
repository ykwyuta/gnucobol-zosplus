       >>SOURCE FORMAT FREE
identification division.
program-id. IMSSMOKE.
data division.
working-storage section.
01 function-code pic x(4) value 'GU  '.
01 commit-rc pic s9(9) comp-5.
01 verify-only pic x.
01 io-pcb.
   05 filler pic x(10).
   05 io-status pic x(2).
   05 filler pic x(20).
01 checkpoint-id pic x(8) value 'CHECK001'.
01 dbpcb.
   05 dbd-name pic x(8) value 'CUSTOMER'.
   05 segment-level pic x(2).
   05 db-status pic x(2).
   05 process-options pic x(4).
   05 reserved-area pic x(4).
   05 segment-name pic x(8).
   05 key-length pic 9(8) comp.
   05 sensitive-count pic 9(8) comp.
   05 key-feedback pic x(20).
01 customer-segment.
   05 customer-id pic s9(9) comp-5.
   05 last-name pic x(50).
   05 filler pic x(225).
01 customer-ssa.
   05 filler pic x(8) value 'CUSTOMER'.
   05 filler pic x value '('.
   05 filler pic x(8) value 'CUSTID  '.
   05 filler pic x(2) value 'EQ'.
   05 ssa-customer-id pic s9(9) comp-5 value 1.
   05 filler pic x value ')'.
procedure division.
    accept verify-only from environment 'COB_IMS_VERIFY_ONLY'
    call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
    if verify-only = '1'
        if db-status = spaces and customer-id = 1 and last-name = 'Updated'
            display 'IMS CUSTOMER REOPEN OK'
        else
            display 'IMS CUSTOMER REOPEN FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if verify-only = '4'
        if db-status = spaces and customer-id = 1 and last-name = 'AutoCommit'
            display 'IMS CUSTOMER AUTO REOPEN OK'
        else
            display 'IMS CUSTOMER AUTO REOPEN FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if verify-only = '9'
        if db-status not = spaces or last-name not = 'AutoCommit'
            display 'IMS CUSTOMER FAILED BATCH SETUP FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'GHU ' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
        if db-status not = spaces
            display 'IMS CUSTOMER FAILED BATCH HOLD FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'FailedBatch' to last-name
        move 'REPL' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment
        if db-status = spaces
            display 'IMS CUSTOMER FAILED BATCH UPDATE OK'
            move 8 to return-code
        else
            display 'IMS CUSTOMER FAILED BATCH REPL FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if verify-only = '7'
        if db-status = spaces and customer-id = 1 and last-name = 'Checkpoint'
            display 'IMS CUSTOMER CHECKPOINT REOPEN OK'
        else
            display 'IMS CUSTOMER CHECKPOINT REOPEN FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if verify-only = '6' or '8'
        if db-status not = spaces
            or (verify-only = '6' and last-name not = 'AutoCommit')
            or (verify-only = '8' and last-name not = 'Checkpoint')
            display 'IMS CUSTOMER SYNC SETUP FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'GHU ' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
        if db-status not = spaces
            display 'IMS CUSTOMER SYNC HOLD FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        if verify-only = '6'
            move 'Checkpoint' to last-name
        else
            move 'RolledBack' to last-name
        end-if
        move 'REPL' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment
        if db-status not = spaces
            display 'IMS CUSTOMER SYNC REPL FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        if verify-only = '6'
            move 'CHKP' to function-code
            call 'CBLTDLI' using function-code io-pcb checkpoint-id
            if io-status = spaces
                display 'IMS CUSTOMER CHKP OK'
            else
                display 'IMS CUSTOMER CHKP FAIL: ' io-status
                move 1 to return-code
            end-if
        else
            move 'ROLB' to function-code
            call 'CBLTDLI' using function-code io-pcb
            if io-status = spaces
                display 'IMS CUSTOMER ROLB OK'
            else
                display 'IMS CUSTOMER ROLB FAIL: ' io-status
                move 1 to return-code
            end-if
        end-if
        goback
    end-if
    if verify-only = '2'
        if db-status not = spaces or last-name not = 'Updated'
            display 'IMS CUSTOMER ROLLBACK SETUP FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'GHU ' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
        if db-status not = spaces
            display 'IMS CUSTOMER ROLLBACK HOLD FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'Uncommitted' to last-name
        move 'REPL' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment
        if db-status = spaces
            display 'IMS CUSTOMER UNCOMMITTED OK'
        else
            display 'IMS CUSTOMER ROLLBACK REPL FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if verify-only = '3'
        if db-status not = spaces or last-name not = 'Updated'
            display 'IMS CUSTOMER AUTO SETUP FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'GHU ' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
        if db-status not = spaces
            display 'IMS CUSTOMER AUTO HOLD FAIL: ' db-status
            move 1 to return-code
            goback
        end-if
        move 'AutoCommit' to last-name
        move 'REPL' to function-code
        call 'CBLTDLI' using function-code dbpcb customer-segment
        if db-status = spaces
            display 'IMS CUSTOMER AUTO UPDATE OK'
        else
            display 'IMS CUSTOMER AUTO REPL FAIL: ' db-status
            move 1 to return-code
        end-if
        goback
    end-if
    if db-status = spaces and customer-id = 1 and last-name = 'Antonelli'
        continue
    else
        display 'IMS CUSTOMER FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move 'GHU ' to function-code
    call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
    if db-status not = spaces
        display 'IMS CUSTOMER HOLD FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    move 'Updated' to last-name
    move 'REPL' to function-code
    call 'CBLTDLI' using function-code dbpcb customer-segment
    if db-status not = spaces
        display 'IMS CUSTOMER REPL FAIL: ' db-status
        move 1 to return-code
        goback
    end-if
    call 'IMSDBCOM' returning commit-rc
    if commit-rc not = 0
        display 'IMS CUSTOMER COMMIT FAIL'
        move 1 to return-code
        goback
    end-if
    move 'GU  ' to function-code
    call 'CBLTDLI' using function-code dbpcb customer-segment customer-ssa
    if db-status = spaces and last-name = 'Updated'
        display 'IMS CUSTOMER UPDATE OK'
    else
        display 'IMS CUSTOMER READBACK FAIL: ' db-status
        move 1 to return-code
    end-if
    goback.
