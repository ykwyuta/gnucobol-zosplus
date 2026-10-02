       identification division.
       program-id. symbolic-smoke.
       data division.
       working-storage section.
       copy BNK1MAI.
       procedure division.
           move low-values to BNK1MEO
           move -1 to ACTIONL
           move 'Hello' to MESSAGEO
           if ACTIONL not = -1
               move 1 to return-code
           end-if
           if MESSAGEO(1:5) not = 'Hello'
               move 2 to return-code
           end-if
           goback.
