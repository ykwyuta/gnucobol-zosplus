#ifndef COBCICS_PG_VSAM_H
#define COBCICS_PG_VSAM_H
#include <stddef.h>

int cobcics_pg_read(const char *table, size_t record_size, size_t key_size,
                    size_t key_offset, const unsigned char *key, size_t compare_size,
                    int gteq, int update, unsigned char *found_key,
                    unsigned char *record);
int cobcics_pg_write(const char *table, size_t record_size, size_t key_size,
                     size_t key_offset, const unsigned char *record);
int cobcics_pg_rewrite(const char *table, size_t record_size, size_t key_size,
                       const unsigned char *key, const unsigned char *record);
int cobcics_pg_delete(const char *table, size_t key_size,
                      const unsigned char *key);
#endif
