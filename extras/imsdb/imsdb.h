#ifndef IMSDB_H
#define IMSDB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ImsHandle ImsHandle;
typedef struct {
    uint8_t destination[8];
    uint8_t source[8];
    const uint8_t *data;
    size_t length;
} ImsTmOutput;
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
/* PSB PCB order for imsbatch: index -1 returns the count. */
int32_t imsdb_psb_layout(const char *psb_file, int32_t index, uint8_t *kind,
                         uint8_t name[8], uint8_t *modifiable,
                         char *error, size_t error_capacity);
/* A durable local IMS TM queue. Destination/source are eight ASCII bytes.
 * imstm_get returns 0 for a message, 1 for an empty queue, -1 on error. */
int32_t imstm_get(const char *path, const uint8_t destination[8], uint8_t *area,
                  size_t capacity, size_t *actual_length, uint8_t source[8], uint64_t *id,
                  char *error, size_t error_capacity);
int32_t imstm_complete(const char *path, const uint64_t *ids, size_t id_count,
                       const ImsTmOutput *outputs, size_t output_count,
                       char *error, size_t error_capacity);
int32_t imstm_rollback(const char *path, const uint64_t *ids, size_t id_count,
                       char *error, size_t error_capacity);

#ifdef __cplusplus
}
#endif
#endif
