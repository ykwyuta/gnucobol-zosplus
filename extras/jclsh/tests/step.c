#include <stdio.h>
#include <stdlib.h>

int main(void) {
    const char *card = getenv("DD_CARD");
    const char *parm = getenv("JCL_PARM");
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
    printf("PARM=%s CARD=%d\n", parm != NULL ? parm : "", rc);
    return rc;
}
