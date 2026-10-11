       >>SOURCE FORMAT FREE
identification division.
program-id. IMSPAUTVERIFY.
data division.
working-storage section.
01 operation pic x(4).
01 db-pcb.
   05 dbd-name pic x(8) value 'DBPAUTP0'.
   05 filler pic x(2).
   05 db-status pic x(2).
   05 filler pic x(44).
01 root-record pic x(100).
01 child-record pic x(200).
01 root-ssa pic x(8) value 'PAUTSUM0'.
01 child-ssa pic x(8) value 'PAUTDTL1'.
01 root-count binary-long signed value zero.
01 child-count binary-long signed value zero.
01 done-flag pic 9 value zero.
procedure division.
    perform until done-flag = 1
        move 'GN  ' to operation
        call 'CBLTDLI' using operation db-pcb root-record root-ssa
        evaluate db-status
          when spaces
            add 1 to root-count
            perform until done-flag = 2
                move 'GNP ' to operation
                call 'CBLTDLI' using operation db-pcb
                    child-record child-ssa
                evaluate db-status
                  when spaces
                    add 1 to child-count
                  when 'GE'
                  when 'GB'
                    move 2 to done-flag
                  when other
                    move 8 to return-code
                    goback
                end-evaluate
            end-perform
            move zero to done-flag
          when 'GB'
            move 1 to done-flag
          when other
            move 8 to return-code
            goback
        end-evaluate
    end-perform
    if root-count not = 22 or child-count not = 202
        display 'IMS CARDDEMO COUNTS FAILED '
            root-count ' ' child-count
        move 8 to return-code
        goback
    end-if
    display 'IMS CARDDEMO LOAD 22 ROOTS 202 CHILDREN OK'
    goback.
