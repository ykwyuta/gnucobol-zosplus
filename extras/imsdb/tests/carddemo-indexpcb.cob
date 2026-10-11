       >>SOURCE FORMAT FREE
identification division.
program-id. IMSPAUTPOINTER.
data division.
working-storage section.
01 operation pic x(4).
01 db-pcb.
   05 dbd-name pic x(8) value 'DBPAUTX0'.
   05 filler pic x(2).
   05 db-status pic x(2).
   05 filler pic x(30).
01 pointer-record pic x(6).
01 pointer-ssa pic x(8) value 'PAUTINDX'.
01 qualified-ssa.
   05 filler pic x(8) value 'PAUTINDX'.
   05 filler pic x value '('.
   05 filler pic x(8) value 'INDXSEQ '.
   05 filler pic x(2) value 'EQ'.
   05 search-key pic x(6).
   05 filler pic x value ')'.
01 first-key pic x(6).
01 pointer-count binary-long signed value zero.
01 done-flag pic 9 value zero.
procedure division.
    perform until done-flag = 1
        move 'GN  ' to operation
        call 'CBLTDLI' using operation db-pcb
            pointer-record pointer-ssa
        evaluate db-status
          when spaces
            if pointer-count = 0
                move pointer-record to first-key
            end-if
            add 1 to pointer-count
          when 'GB'
            move 1 to done-flag
          when other
            move 8 to return-code
            goback
        end-evaluate
    end-perform
    if pointer-count not = 22
        move 8 to return-code
        goback
    end-if
    move first-key to search-key
    move 'GU  ' to operation
    call 'CBLTDLI' using operation db-pcb
        pointer-record qualified-ssa
    if db-status not = spaces or pointer-record not = first-key
        move 8 to return-code
        goback
    end-if
    display 'IMS CARDDEMO INDEX PCB 22 POINTERS OK'
    goback.
