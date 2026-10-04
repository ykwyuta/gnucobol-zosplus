#ifndef PLI_RUNTIME_H
#define PLI_RUNTIME_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#define PLI_MAX_CHAR 512

typedef enum { PLI_CHAR, PLI_BIN, PLI_DEC, PLI_BIT } pli_kind;
typedef struct {
    pli_kind kind;
    int precision;
    int scale;
    size_t length;
    int64_t number;
    char chars[PLI_MAX_CHAR];
} pli_value;

typedef struct {
    FILE *stream;
    size_t record_size;
} pli_file;

int pli_file_open(pli_file *file, const char *dd_name, size_t record_size);
int pli_file_read(pli_file *file, pli_value *target);
void pli_file_close(pli_file *file);

pli_value pli_storage_char(size_t length);
pli_value pli_storage_num(pli_kind kind, int precision, int scale);
pli_value pli_literal(const char *text);
pli_value pli_number(const char *text);
pli_value pli_bit(int value);
void pli_assign(pli_value *target, pli_value source);
pli_value pli_add(pli_value a, pli_value b);
pli_value pli_sub(pli_value a, pli_value b);
pli_value pli_mul(pli_value a, pli_value b);
pli_value pli_div(pli_value a, pli_value b);
pli_value pli_neg(pli_value a);
pli_value pli_concat(pli_value a, pli_value b);
pli_value pli_substr(pli_value a, pli_value start, pli_value length);
pli_value pli_trim(pli_value a);
pli_value pli_centre(pli_value a, pli_value width);
pli_value pli_char_builtin(pli_value a);
pli_value pli_format_fixed(pli_value a, size_t width, int decimals);
pli_value pli_abs(pli_value a);
pli_value pli_datetime(void);
pli_value pli_eq(pli_value a, pli_value b);
pli_value pli_ne(pli_value a, pli_value b);
pli_value pli_lt(pli_value a, pli_value b);
pli_value pli_le(pli_value a, pli_value b);
pli_value pli_gt(pli_value a, pli_value b);
pli_value pli_ge(pli_value a, pli_value b);
pli_value pli_and(pli_value a, pli_value b);
pli_value pli_or(pli_value a, pli_value b);
pli_value pli_not(pli_value a);
int pli_truth(pli_value a);
void pli_put_skip_list(size_t count, const pli_value *items);
void pli_put_skip_edit(pli_value item);
void pli_put_page(void);

#endif
