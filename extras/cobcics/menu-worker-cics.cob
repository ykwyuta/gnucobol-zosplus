       IDENTIFICATION DIVISION.
       PROGRAM-ID. MENUCICS.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 COMMUNICATION-AREA PIC X.
       01 WS-CICS-RESP PIC S9(8) COMP.
       01 WS-CICS-RESP2 PIC S9(8) COMP.
       01 WS-APPLID PIC X(8).
       01 WS-PROGRAM PIC X(8).
       01 WS-LINK-PGM PIC X(8) VALUE 'MENULINK'.
       01 WS-END-MESSAGE PIC X(13) VALUE 'Session Ended'.
       01 WS-U-TIME PIC S9(15) COMP-3.
       01 WS-DATE PIC X(10).
       01 WS-TIME PIC X(6).
       COPY BNK1MAI.
       COPY DFHAID.
       PROCEDURE DIVISION.
           EXEC CICS ASSIGN APPLID(WS-APPLID)
             PROGRAM(WS-PROGRAM) END-EXEC.
           MOVE LOW-VALUES TO BNK1MEO.
           IF EIBCALEN > ZERO
               EXEC CICS RECEIVE MAP('BNK1ME')
                 MAPSET('BNK1MAI') INTO(BNK1MEI)
                 RESP(WS-CICS-RESP) RESP2(WS-CICS-RESP2)
               END-EXEC
           END-IF.
           IF EIBAID = DFHCLEAR AND
              WS-CICS-RESP = DFHRESP(MAPFAIL)
               EXEC CICS SEND CONTROL ERASE FREEKB END-EXEC
               EXEC CICS RETURN END-EXEC
           END-IF.
           IF EIBAID = DFHENTER AND ACTIONI = '3'
               EXEC CICS SEND TEXT FROM(WS-END-MESSAGE)
                 ERASE FREEKB RESP(WS-CICS-RESP)
                 RESP2(WS-CICS-RESP2) END-EXEC
               EXEC CICS RETURN END-EXEC
           END-IF.
           IF EIBAID = DFHENTER AND ACTIONI = '2'
               MOVE 'I' TO COMMUNICATION-AREA
               EXEC CICS RETURN TRANSID('ODCS')
                 COMMAREA(COMMUNICATION-AREA) LENGTH(1) IMMEDIATE
               END-EXEC
           END-IF.
           IF EIBAID = DFHENTER AND
              (ACTIONI = '4' OR ACTIONI = '6')
               IF ACTIONI = '4'
                   EXEC CICS ASKTIME ABSTIME(WS-U-TIME)
                   END-EXEC
               ELSE
                   MOVE 3160816496000 TO WS-U-TIME
               END-IF
               EXEC CICS FORMATTIME ABSTIME(WS-U-TIME)
                 DDMMYYYY(WS-DATE) TIME(WS-TIME) DATESEP
               END-EXEC
               MOVE SPACES TO MESSAGEO
               STRING 'COBOL CLOCK ' WS-DATE ' ' WS-TIME
                 DELIMITED BY SIZE INTO MESSAGEO
               END-STRING
               EXEC CICS SEND MAP('BNK1ME')
                 MAPSET('BNK1MAI') FROM(BNK1MEO)
                 ERASE FREEKB END-EXEC
               EXEC CICS RETURN END-EXEC
           END-IF.
           IF EIBAID = DFHENTER AND ACTIONI = '5'
               EXEC CICS ABEND ABCODE('HBNK') NODUMP
               END-EXEC
           END-IF.
           MOVE SPACES TO MESSAGEO.
           EVALUATE TRUE
             WHEN EIBCALEN = ZERO
               IF EIBTRNID = 'OMEN'
                   STRING 'COBOL MENU READY ' WS-APPLID ' '
                     WS-PROGRAM DELIMITED BY SIZE
                     INTO MESSAGEO END-STRING
               ELSE
                   MOVE 'BAD INITIAL TRANSID' TO MESSAGEO
               END-IF
             WHEN EIBAID = DFHENTER AND
                  WS-CICS-RESP = DFHRESP(NORMAL)
               MOVE 'Y' TO COMMUNICATION-AREA
               EXEC CICS LINK PROGRAM(WS-LINK-PGM)
                 COMMAREA(COMMUNICATION-AREA)
                 RESP(WS-CICS-RESP) RESP2(WS-CICS-RESP2)
               END-EXEC
               STRING 'COBOL MENU ACTION ' ACTIONI ' '
                 COBCICS-COMMAREA(1:1) ' '
                 COMMUNICATION-AREA
                 DELIMITED BY SIZE INTO MESSAGEO
               END-STRING
             WHEN EIBAID = DFHPF3
               MOVE 'COBOL AID F3' TO MESSAGEO
             WHEN WS-CICS-RESP = DFHRESP(MAPFAIL)
               MOVE 'COBOL MAPFAIL 36' TO MESSAGEO
             WHEN OTHER
               MOVE 'COBOL OTHER AID' TO MESSAGEO
           END-EVALUATE.
           EVALUATE TRUE
             WHEN EIBCALEN = ZERO
               EXEC CICS SEND MAP('BNK1ME')
                 MAPSET('BNK1MAI') FROM(BNK1MEO)
                 ERASE FREEKB
               END-EXEC
             WHEN WS-CICS-RESP = DFHRESP(MAPFAIL)
               EXEC CICS SEND MAP('BNK1ME')
                 MAPSET('BNK1MAI') FROM(BNK1MEO)
                 DATAONLY ALARM FREEKB
               END-EXEC
             WHEN OTHER
               EXEC CICS SEND MAP('BNK1ME')
                 MAPSET('BNK1MAI') FROM(BNK1MEO)
                 DATAONLY FREEKB
               END-EXEC
           END-EVALUATE.
           IF EIBAID = DFHPF3
               EXEC CICS RETURN END-EXEC
           ELSE
               MOVE 'Y' TO COMMUNICATION-AREA
               EXEC CICS RETURN TRANSID('OMEN')
                 COMMAREA(COMMUNICATION-AREA) LENGTH(1)
                 RESP(WS-CICS-RESP) RESP2(WS-CICS-RESP2)
               END-EXEC
           END-IF.
