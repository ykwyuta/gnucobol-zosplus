#ifndef COBCICS_LOCAL_VSAM_H
#define COBCICS_LOCAL_VSAM_H
#include <stddef.h>

int cobcics_vsam_layout(const char *path, size_t *record_size,
                        size_t *key_size, size_t *key_offset);
int cobcics_vsam_begin(void);
int cobcics_vsam_prepare(void);
int cobcics_vsam_finish(int commit);
int cobcics_vsam_read_update(const char *path, size_t record_size, size_t key_size,
                             size_t key_offset, const unsigned char *key,
                             size_t compare_size, int gteq, unsigned char *record);
int cobcics_vsam_read(const char *path, size_t record_size, size_t key_size,
                      size_t key_offset, const unsigned char *key,
                      size_t compare_size, int gteq, unsigned char *record);
int cobcics_vsam_write(const char *path, size_t record_size, size_t key_size,
                       size_t key_offset, const unsigned char *record);
int cobcics_vsam_rewrite(const char *path, size_t record_size, size_t key_size,
                         size_t key_offset, const unsigned char *key,
                         const unsigned char *record);
int cobcics_vsam_delete(const char *path, size_t record_size, size_t key_size,
                        size_t key_offset, const unsigned char *key);
#endif
