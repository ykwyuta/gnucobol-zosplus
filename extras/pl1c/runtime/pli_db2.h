#ifndef PLI_DB2_H
#define PLI_DB2_H

#include "pli_runtime.h"

typedef struct {
    pli_value *value;
    pli_value *indicator;
} pli_sql_param;

void pli_db2_open(pli_value *code, pli_value *state, const char *name,
                  const char *sql, size_t input_count, const pli_sql_param *inputs);
void pli_db2_fetch(pli_value *code, pli_value *state, const char *name,
                   size_t output_count, const pli_sql_param *outputs);
void pli_db2_close(pli_value *code, pli_value *state, const char *name);
void pli_db2_select(pli_value *code, pli_value *state, const char *sql,
                    size_t input_count, const pli_sql_param *inputs,
                    size_t output_count, const pli_sql_param *outputs);
void pli_db2_commit(pli_value *code, pli_value *state);

#endif
