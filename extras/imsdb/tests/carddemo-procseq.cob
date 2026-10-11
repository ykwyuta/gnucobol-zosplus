       >>SOURCE FORMAT FREE
identification division.
program-id. IMSPAUTINDEX.
data division.
working-storage section.
01 operation pic x(4).
01 db-pcb.
   05 dbd-name pic x(8) value 'DBPAUTP0'.
   05 filler pic x(2).
   05 db-status pic x(2).
   05 filler pic x(44).
01 root-record pic x(100).
01 root-ssa pic x(8) value 'PAUTSUM0'.
01 qualified-ssa.
   05 filler pic x(8) value 'PAUTSUM0'.
   05 filler pic x value '('.
   05 filler pic x(8) value 'ACCNTID '.
   05 filler pic x(2) value 'EQ'.
   05 search-key pic x(6).
   05 filler pic x value ')'.
01 first-key pic x(6).
01 root-count binary-long signed value zero.
01 done-flag pic 9 value zero.
procedure division.
    perform until done-flag = 1
        move 'GN  ' to operation
        call 'CBLTDLI' using operation db-pcb root-record root-ssa
        evaluate db-status
          when spaces
            if root-count = 0
                move root-record(1:6) to first-key
            end-if
            add 1 to root-count
          when 'GB'
            move 1 to done-flag
          when other
            move 8 to return-code
            goback
        end-evaluate
    end-perform
    if root-count not = 22
        move 8 to return-code
        goback
    end-if
    move first-key to search-key
    move 'GU  ' to operation
    call 'CBLTDLI' using operation db-pcb root-record qualified-ssa
    if db-status not = spaces or root-record(1:6) not = first-key
        move 8 to return-code
        goback
    end-if
    display 'IMS CARDDEMO PROCSEQ 22 ROOTS OK'
    goback.
