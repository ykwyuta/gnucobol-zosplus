       >>SOURCE FORMAT FREE
identification division.
program-id. IMSTMINPUT.
data division.
working-storage section.
01 gu-code pic x(4) value 'GU  '.
01 gn-code pic x(4) value 'GN  '.
01 input-message.
   05 message-length pic 9(4) comp.
   05 message-control pic x(2).
   05 message-body pic x(4).
01 fail-mode pic x.
linkage section.
01 io-pcb.
   05 terminal-name pic x(8).
   05 filler pic x(2).
   05 io-status pic x(2).
   05 filler pic x(20).
procedure division using io-pcb.
    call 'CBLTDLI' using gu-code io-pcb input-message
    if io-status not = spaces or terminal-name not = 'TERM0001'
        or message-body not = 'ABCD'
        display 'GU FAIL ' io-status ' ' terminal-name ' ' message-body
        move 12 to return-code
        goback
    end-if
    call 'CBLTDLI' using gn-code io-pcb input-message
    if io-status not = spaces or message-body not = 'EFGH'
        display 'GN FAIL ' io-status ' ' message-body
        move 12 to return-code
        goback
    end-if
    call 'CBLTDLI' using gn-code io-pcb input-message
    if io-status not = 'QD'
        display 'GN END FAIL ' io-status
        move 12 to return-code
    else
        accept fail-mode from environment 'IMS_TM_FAIL'
        if fail-mode = '1'
            move 8 to return-code
        else
            move 0 to return-code
        end-if
    end-if
    goback.
