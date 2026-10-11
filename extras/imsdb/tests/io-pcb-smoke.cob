       >>SOURCE FORMAT FREE
identification division.
program-id. IMSIOTEST.
data division.
working-storage section.
01 function-code pic x(4) value 'ISRT'.
01 io-pcb pic x(32).
01 output-message.
   05 message-length pic 9(4) comp value 8.
   05 message-control pic x(2) value low-values.
   05 message-text pic x(4) value 'TEST'.
01 fail-mode pic x value '0'.
procedure division.
    call 'CBLTDLI' using function-code io-pcb output-message
    if io-pcb(11:2) not = spaces
        display 'IMS IO PCB INSERT FAIL: ' io-pcb(11:2)
        move 12 to return-code
        stop run
    end-if
    accept fail-mode from environment 'IMS_IO_FAIL'
    if fail-mode = '1'
        move 8 to return-code
    else
        move 0 to return-code
    end-if
    stop run.
