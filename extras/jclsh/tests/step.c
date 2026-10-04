#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    const char *card = getenv("DD_CARD");
    const char *input_path = getenv("DD_INPUT");
    const char *output_path = getenv("DD_OUTPUT");
    const char *parm = getenv("JCL_PARM");
    const char *abend_path = getenv("JCL_ABEND_FILE");
    int rc = 0;
    if (card != NULL) {
        FILE *input = fopen(card, "r");
        if (input == NULL || fscanf(input, "%d", &rc) != 1) {
            fprintf(stderr, "CARD read failed\n");
            if (input != NULL) fclose(input);
            return 12;
        }
        fclose(input);
    }
    if (input_path != NULL) {
        char value[128] = {0};
        FILE *input = fopen(input_path, "r");
        if (input == NULL || fgets(value, sizeof value, input) == NULL) {
            fprintf(stderr, "INPUT read failed\n");
            if (input != NULL) fclose(input);
            return 12;
        }
        fclose(input);
        printf("INPUT=%s", value);
    }
    if (output_path != NULL) {
        FILE *output = fopen(output_path, "w");
        if (output == NULL || fprintf(output, "%s\n", parm != NULL ? parm : "") < 0) {
            fprintf(stderr, "OUTPUT write failed\n");
            if (output != NULL) fclose(output);
            return 12;
        }
        fclose(output);
    }
    printf("PARM=%s CARD=%d\n", parm != NULL ? parm : "", rc);
    if (parm != NULL && strcmp(parm, "ABEND") == 0) {
        FILE *abend = abend_path != NULL ? fopen(abend_path, "w") : NULL;
        if (abend == NULL || fputs("U1234\n", abend) == EOF) {
            fprintf(stderr, "ABEND marker write failed\n");
            if (abend != NULL) fclose(abend);
            return 12;
        }
        fclose(abend);
        return 12;
    }
    return rc;
}
