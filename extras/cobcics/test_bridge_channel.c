/* Exercise the CICS channel, child response, and ENQ bridge ABI directly. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

void COBCICS_START(unsigned char *, unsigned char *, unsigned char *);
void COBCICS_PUT_CONTAINER(const char *, const unsigned char *,
                          const unsigned char *, const unsigned char *,
                          const char *, unsigned char *, unsigned char *);
void COBCICS_GET_CONTAINER(const char *, unsigned char *, unsigned char *,
                          const unsigned char *, const char *,
                          unsigned char *, unsigned char *);
void COBCICS_RUN_ENTER(const char *, const char *, char *, char *,
                      unsigned char *, unsigned char *);
void COBCICS_RUN_EXIT(void);
void COBCICS_FETCH_ANY(char *, char *, unsigned char *, char *,
                       unsigned char *, unsigned char *);
void COBCICS_ENQ(const unsigned char *, const unsigned char *,
                 const unsigned char *, unsigned char *, unsigned char *);
void COBCICS_DEQ(const unsigned char *, const unsigned char *,
                 const unsigned char *, unsigned char *, unsigned char *);

static uint32_t number(const unsigned char *value) {
    return (uint32_t)value[0] << 24 | (uint32_t)value[1] << 16 |
           (uint32_t)value[2] << 8 | value[3];
}

int main(void) {
    unsigned char aid, calen[2], transid[4], resp[4], resp2[4];
    unsigned char one[4] = {0, 0, 0, 1};
    unsigned char result[2] = {0};
    unsigned char status[4];
    unsigned char size[4] = {0, 0, 0, 2};
    unsigned char lock_size[2] = {0, 4};
    char token[16], fetched[16], reply_channel[16], program[8], abcode[4];
    const char channel[16] = "CIPCREDCHANN    ";
    const char container[16] = "CIPA            ";
    COBCICS_START(&aid, calen, transid);
    assert(aid == 0x7d && number((unsigned char[]){0, 0, calen[0], calen[1]}) == 0);
    COBCICS_PUT_CONTAINER(container, (const unsigned char *)"A", one,
                         one, channel, resp, resp2);
    assert(number(resp) == 0);
    COBCICS_RUN_ENTER("OCR1", channel, token, program, resp, resp2);
    assert(number(resp) == 0 && memcmp(program, "CRDTAGY1", 8) == 0);
    size[3] = 2;
    COBCICS_GET_CONTAINER(container, result, size, size, channel, resp, resp2);
    assert(number(resp) == 0 && result[0] == 'A');
    COBCICS_PUT_CONTAINER(container, (const unsigned char *)"B", one,
                         one, channel, resp, resp2);
    assert(number(resp) == 0);
    COBCICS_RUN_EXIT();
    COBCICS_FETCH_ANY(fetched, reply_channel, status, abcode, resp, resp2);
    assert(number(resp) == 0 && number(status) == 1016);
    assert(memcmp(token, fetched, 16) == 0);
    size[3] = 2;
    COBCICS_GET_CONTAINER(container, result, size, size, reply_channel, resp, resp2);
    assert(number(resp) == 0 && result[0] == 'B');
    size[3] = 2;
    COBCICS_GET_CONTAINER(container, result, size, size, channel, resp, resp2);
    assert(number(resp) == 0 && result[0] == 'A');
    COBCICS_FETCH_ANY(fetched, reply_channel, status, abcode, resp, resp2);
    assert(number(resp) == 13);
    COBCICS_ENQ((const unsigned char *)"LOCK", lock_size, lock_size, resp, resp2);
    assert(number(resp) == 0);
    COBCICS_DEQ((const unsigned char *)"LOCK", lock_size, lock_size, resp, resp2);
    assert(number(resp) == 0);
    puts("CICS channel and lock bridge OK");
    return 0;
}
