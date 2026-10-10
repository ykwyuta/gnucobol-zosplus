       >>SOURCE FORMAT FREE
identification division.
program-id. MQDEMO.
data division.
working-storage section.
01 hconn pic s9(9) comp value 0.
01 hobj pic s9(9) comp value 0.
01 opts pic s9(9) comp value 0.
01 cc pic s9(9) comp value 0.
01 reason pic s9(9) comp value 0.
01 buffer-length pic s9(9) comp value 8.
01 data-length pic s9(9) comp value 0.
01 message-buffer pic x(80) value 'HELLO MQ'.
01 object-descriptor.
   copy 'CMQODV.cpy'.
01 message-descriptor.
   copy 'CMQMDV.cpy'.
01 put-options.
   copy 'CMQPMOV.cpy'.
01 get-options.
   copy 'CMQGMOV.cpy'.
procedure division.
    display 'MQ START'
    call 'MQCONN' using 'QM1' hconn cc reason
    perform check-result
    display 'MQ CONNECTED'
    move 'DEV.COBMQ.DEMO' to MQOD-OBJECTNAME
    call 'MQOPEN' using hconn object-descriptor opts hobj cc reason
    perform check-result
    display 'MQ OPENED'
    call 'MQPUT' using hconn hobj message-descriptor put-options
        buffer-length message-buffer cc reason
    perform check-result
    display 'MQ PUT DONE'
    move spaces to message-buffer
    call 'MQGET' using hconn hobj message-descriptor get-options
        buffer-length message-buffer data-length cc reason
    perform check-result
    display 'MQ GET DONE'
    if data-length not = 8 or message-buffer(1:8) not = 'HELLO MQ'
        display 'MQ PAYLOAD MISMATCH: ' message-buffer
        move 1 to return-code
        goback
    end-if
    move 'PUT1 MQ!' to message-buffer
    call 'MQPUT1' using hconn object-descriptor message-descriptor
        put-options buffer-length message-buffer cc reason
    perform check-result
    move spaces to message-buffer
    call 'MQGET' using hconn hobj message-descriptor get-options
        buffer-length message-buffer data-length cc reason
    perform check-result
    if data-length not = 8 or message-buffer(1:8) not = 'PUT1 MQ!'
        display 'MQPUT1 PAYLOAD MISMATCH: ' message-buffer
        move 1 to return-code
        goback
    end-if
    call 'MQGET' using hconn hobj message-descriptor get-options
        buffer-length message-buffer data-length cc reason
    if cc not = 2 or reason not = 2033
        display 'MQ EMPTY QUEUE REASON MISMATCH: ' reason
        move 1 to return-code
        goback
    end-if
    call 'MQCLOSE' using hconn hobj opts cc reason
    perform check-result
    call 'MQDISC' using hconn cc reason
    perform check-result
    display 'MQ ROUNDTRIP OK'
    goback.
check-result.
    if cc not = 0
        display 'MQ ERROR: ' reason
        move 1 to return-code
        stop run
    end-if.
