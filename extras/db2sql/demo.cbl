       IDENTIFICATION DIVISION.
       PROGRAM-ID. DB2DEMO.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 HV-ID PIC X(8) VALUE 'A0000001'.
       01 HV-AMOUNT PIC S9(9) COMP VALUE 42.
       01 HV-FOUND PIC S9(9) COMP VALUE 0.
           EXEC SQL INCLUDE SQLCA END-EXEC.
       PROCEDURE DIVISION.
           EXEC SQL
             INSERT INTO DB2SQL_DEMO (ID, AMOUNT)
             VALUES (:HV-ID, :HV-AMOUNT)
           END-EXEC.
           IF SQLCODE NOT = 0
               DISPLAY 'INSERT FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           EXEC SQL
             SELECT AMOUNT INTO :HV-FOUND
             FROM DB2SQL_DEMO WHERE ID = :HV-ID
           END-EXEC.
           IF SQLCODE NOT = 0 OR HV-FOUND NOT = 42
               DISPLAY 'SELECT FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           MOVE 'A0000002' TO HV-ID.
           EXEC SQL
             INSERT INTO DB2SQL_DEMO (ID, AMOUNT)
             VALUES (:HV-ID, :HV-AMOUNT)
           END-EXEC.
           IF SQLCODE NOT = 0
               DISPLAY 'SECOND INSERT FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           EXEC SQL
             SELECT AMOUNT INTO :HV-FOUND FROM DB2SQL_DEMO
           END-EXEC.
           IF SQLCODE NOT = -811 OR SQLSTATE NOT = '21000'
               DISPLAY 'MULTI ROW FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           MOVE 'MISSING' TO HV-ID.
           EXEC SQL
             SELECT AMOUNT INTO :HV-FOUND
             FROM DB2SQL_DEMO WHERE ID = :HV-ID
           END-EXEC.
           IF SQLCODE NOT = 100 OR SQLSTATE NOT = '02000'
               DISPLAY 'NO DATA FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           MOVE 'A0000001' TO HV-ID.
           EXEC SQL
             INSERT INTO DB2SQL_DEMO (ID, AMOUNT)
             VALUES (:HV-ID, :HV-AMOUNT)
           END-EXEC.
           IF SQLCODE NOT = -803 OR SQLSTATE NOT = '23505'
               DISPLAY 'ERROR FAILED ' SQLCODE ' ' SQLSTATE
               MOVE 1 TO RETURN-CODE
               STOP RUN
           END-IF.
           DISPLAY 'DB2 SQL OK ' HV-FOUND.
           MOVE 0 TO RETURN-CODE.
           STOP RUN.
