       >>SOURCE FORMAT FREE
identification division.
program-id. IMSUOWT.
data division.
working-storage section.
01 operation pic x(4) value 'GU  '.
01 test-mode pic x.
01 sync-rc binary-long signed.
01 absolute-time pic s9(15) comp-3.
01 absolute-length pic s9(4) comp value 8.
01 formatted-date pic x(6).
01 formatted-clock pic x(6).
01 date-length pic s9(4) comp value 6.
01 clock-length pic s9(4) comp value 6.
01 clock-milliseconds pic s9(8) comp.
01 time-response pic s9(9) comp.
01 time-response2 pic s9(9) comp.
01 db-pcb.
   05 dbd-name pic x(8) value 'CUSTOMER'.
   05 filler pic x(2).
   05 db-status pic x(2).
   05 filler pic x(44).
01 customer-record.
   05 customer-id pic s9(9) comp-5.
   05 customer-last pic x(50).
   05 filler pic x(225).
01 customer-ssa.
   05 filler pic x(8) value 'CUSTOMER'.
   05 filler pic x value '('.
   05 filler pic x(8) value 'CUSTID  '.
   05 filler pic x(2) value 'EQ'.
   05 sought-id pic s9(9) comp-5 value 1.
   05 filler pic x value ')'.
procedure division.
    call static 'TESTCICSSTART'
    call static 'COBCICS_ASKTIME'
        using absolute-time absolute-length
    call static 'COBCICS_FORMATTIME_YYDDD'
        using absolute-time absolute-length
              formatted-date date-length
              formatted-clock clock-length
              clock-milliseconds time-response time-response2
    if time-response not = zero or
        formatted-date(1:5) is not numeric or
        formatted-clock is not numeric or
        clock-milliseconds < zero or
        clock-milliseconds > 999
        move 8 to return-code
        goback
    end-if
    call static 'COBCICS_FORMATTIME_YYMMDD'
        using absolute-time absolute-length
              formatted-date date-length
              formatted-clock clock-length
              time-response time-response2
    if time-response not = zero or
        formatted-date is not numeric
        move 8 to return-code
        goback
    end-if
    accept test-mode from environment 'COB_IMS_MODE'
    call 'CBLTDLI' using operation db-pcb
        customer-record customer-ssa
    if db-status not = spaces or customer-id not = 1
        move 8 to return-code
        goback
    end-if
    if test-mode = 'V'
        if customer-last not = 'CicsCommit'
            move 8 to return-code
            goback
        end-if
        display 'IMS CICS UOW REOPEN OK'
        goback
    end-if
    move 'GHU ' to operation
    call 'CBLTDLI' using operation db-pcb
        customer-record customer-ssa
    if db-status not = spaces
        move 8 to return-code
        goback
    end-if
    if test-mode = 'C'
        move 'CicsCommit' to customer-last
    else
        move 'CicsRollback' to customer-last
    end-if
    move 'REPL' to operation
    call 'CBLTDLI' using operation db-pcb customer-record
    if db-status not = spaces
        move 8 to return-code
        goback
    end-if
    if test-mode = 'C'
        call static 'TESTIMSCOM' returning sync-rc
    else
        call static 'TESTIMSROL' returning sync-rc
    end-if
    if sync-rc not = zero
        move 8 to return-code
        goback
    end-if
    display 'IMS CICS UOW ' test-mode ' OK'
    goback.
