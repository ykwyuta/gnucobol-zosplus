#include "bridge.c"

int main(void) {
    unsigned char abstime[8], date[8], clock[8], resp[4], resp2[4];
    const unsigned char abstime_length[2] = {0, 8};
    const unsigned char date_length[2] = {0, 8};
    const unsigned char short_date_length[2] = {0, 6};
    const unsigned char clock_length[2] = {0, 8};
    uint64_t days = (uint64_t)(days_from_civil(2026, 10, 12) + 25567);
    pack_abstime(abstime, days * 86400000ULL + 15 * 3600000ULL + 23 * 60000ULL);
    started = 1;
    COBCICS_FORMATTIME_MMDDYY(abstime, abstime_length, date, date_length,
                               clock, clock_length, (const unsigned char *)"/",
                               resp, resp2);
    if (memcmp(date, "10/12/26", 8) || memcmp(clock, "152300  ", 8) ||
        cobol_fullword(resp) || cobol_fullword(resp2))
        return 1;
    COBCICS_FORMATTIME_MMDDYY_TIMESEP(
        abstime, abstime_length, date, date_length, clock, clock_length,
        (const unsigned char *)"/", (const unsigned char *)":", resp, resp2);
    if (memcmp(date, "10/12/26", 8) || memcmp(clock, "15:23:00", 8) ||
        cobol_fullword(resp) || cobol_fullword(resp2))
        return 1;
    COBCICS_FORMATTIME_MMDDYY(abstime, abstime_length, date,
                               short_date_length, clock, clock_length,
                               (const unsigned char *)"", resp, resp2);
    if (memcmp(date, "101226", 6) || cobol_fullword(resp) ||
        cobol_fullword(resp2))
        return 1;
    puts("CICS FORMATTIME MMDDYY OK");
    return 0;
}
