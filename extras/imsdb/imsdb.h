#ifndef IMSDB_H
#define IMSDB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ImsHandle ImsHandle;
typedef struct {
    uint8_t *data; /* DB SSA input; GSAM RSA input/output. */
    size_t len;
} ImsSpan;

/* encoding flags: bit 0 = GnuCOBOL 1047 EBCDIC controls; bit 1 = native little-endian COMP-5. */
ImsHandle *imsdb_open(const char *wal, const char *dbd_dir, const char *psb_file,
                      uint32_t encoding, char *error, size_t error_capacity);
int32_t imsdb_pcb_index(const ImsHandle *handle, const uint8_t *dbd_name, size_t name_len);
/* Return 0 for a completed call, even when the PCB has a nonblank IMS status. */
int32_t imsdb_call(ImsHandle *handle, uint32_t pcb_index,
                   const uint8_t *function, size_t function_len,
                   uint8_t *pcb, size_t pcb_len, uint8_t *io, size_t io_len,
                   const ImsSpan *ssas, size_t ssa_count,
                   char *error, size_t error_capacity);
int32_t imsdb_commit(ImsHandle *handle, char *error, size_t error_capacity);
int32_t imsdb_checkpoint(ImsHandle *handle, const uint8_t id[8],
                         char *error, size_t error_capacity);
void imsdb_rollback(ImsHandle *handle);
void imsdb_close(ImsHandle *handle);

#ifdef __cplusplus
}
#endif
#endif
