#include "pli_runtime.h"

#include <inttypes.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static void fail(const char *message)
{
    fprintf(stderr, "PL/I runtime: %s\n", message);
    exit(EXIT_FAILURE);
}

int pli_file_open(pli_file *file, const char *dd_name, size_t record_size)
{
    char key[128];
    const char *path;
    if (record_size > PLI_MAX_CHAR ||
        snprintf(key, sizeof key, "DD_%s", dd_name) >= (int)sizeof key)
        fail("invalid FILE declaration");
    path = getenv(key);
    if (!path || !*path) return 0;
    file->stream = fopen(path, "rb");
    file->record_size = record_size;
    return file->stream != NULL;
}

int pli_file_read(pli_file *file, pli_value *target)
{
    char buffer[PLI_MAX_CHAR + 2];
    size_t n;
    if (!file->stream || !fgets(buffer, sizeof buffer, file->stream)) return 0;
    n = strlen(buffer);
    while (n && (buffer[n - 1] == '\n' || buffer[n - 1] == '\r')) --n;
    if (n > file->record_size) fail("FILE record exceeds RECSIZE");
    buffer[n] = '\0';
    pli_assign(target, pli_literal(buffer));
    return 1;
}

void pli_file_close(pli_file *file)
{
    if (file->stream) fclose(file->stream);
    file->stream = NULL;
}

static int64_t power10(int n)
{
    int64_t result = 1;
    if (n < 0 || n > 17) fail("unsupported decimal scale");
    while (n--) result *= 10;
    return result;
}

static int64_t checked_add(int64_t a, int64_t b)
{
    if ((b > 0 && a > INT64_MAX - b) || (b < 0 && a < INT64_MIN - b))
        fail("numeric overflow");
    return a + b;
}

static int64_t checked_mul(int64_t a, int64_t b)
{
    if (!a || !b) return 0;
    if ((a == INT64_MIN && b != 1) || (b == INT64_MIN && a != 1))
        fail("numeric overflow");
    if (a > 0 && b > 0 && a > INT64_MAX / b) fail("numeric overflow");
    if (a > 0 && b < 0 && b < INT64_MIN / a) fail("numeric overflow");
    if (a < 0 && b > 0 && a < INT64_MIN / b) fail("numeric overflow");
    if (a < 0 && b < 0 && a < INT64_MAX / b) fail("numeric overflow");
    return a * b;
}

pli_value pli_storage_char(size_t length)
{
    pli_value v = {0};
    if (length > PLI_MAX_CHAR) fail("CHAR length exceeds implementation limit");
    v.kind = PLI_CHAR;
    v.length = length;
    memset(v.chars, ' ', length);
    return v;
}

pli_value pli_storage_num(pli_kind kind, int precision, int scale)
{
    pli_value v = {0};
    if (precision < 1 ||
        (kind == PLI_BIN ? precision > 62 : precision > 17) ||
        scale < 0 || scale > precision)
        fail("unsupported numeric precision");
    v.kind = kind;
    v.precision = precision;
    v.scale = scale;
    return v;
}

pli_value pli_literal(const char *text)
{
    size_t length = strlen(text);
    pli_value v = pli_storage_char(length);
    memcpy(v.chars, text, length);
    return v;
}

pli_value pli_number(const char *text)
{
    pli_value v = pli_storage_num(PLI_DEC, 17, 0);
    int sign = 1, seen_point = 0, digits = 0;
    if (*text == '-') { sign = -1; ++text; }
    else if (*text == '+') ++text;
    for (; *text; ++text) {
        if (*text == '.' && !seen_point) seen_point = 1;
        else if (*text >= '0' && *text <= '9') {
            v.number = checked_add(checked_mul(v.number, 10), *text - '0');
            if (seen_point) ++v.scale;
            ++digits;
        } else fail("invalid numeric literal");
    }
    if (!digits) fail("invalid numeric literal");
    v.number *= sign;
    return v;
}

pli_value pli_bit(int value)
{
    pli_value v = pli_storage_num(PLI_BIT, 1, 0);
    v.number = value != 0;
    return v;
}

static int64_t scaled(pli_value v, int scale)
{
    if (v.kind == PLI_CHAR) fail("character value used as number");
    if (scale < v.scale) return v.number / power10(v.scale - scale);
    return checked_mul(v.number, power10(scale - v.scale));
}

void pli_assign(pli_value *target, pli_value source)
{
    if (target->kind == PLI_CHAR) {
        size_t n;
        if (source.kind != PLI_CHAR) source = pli_char_builtin(source);
        n = source.length < target->length ? source.length : target->length;
        memset(target->chars, ' ', target->length);
        memcpy(target->chars, source.chars, n);
    } else {
        int64_t number = scaled(source, target->scale);
        if (target->kind == PLI_BIT) number = number != 0;
        if (target->kind == PLI_BIN &&
            (number < -(INT64_C(1) << target->precision) ||
             number >= (INT64_C(1) << target->precision)))
            fail("binary assignment exceeds precision");
        if (target->kind == PLI_DEC &&
            (number <= -power10(target->precision) ||
             number >= power10(target->precision)))
            fail("numeric assignment exceeds precision");
        target->number = number;
    }
}

static pli_value numeric(int64_t number, int scale)
{
    pli_value result = pli_storage_num(PLI_DEC, 17, scale);
    result.number = number;
    return result;
}

pli_value pli_add(pli_value a, pli_value b)
{
    int scale = a.scale > b.scale ? a.scale : b.scale;
    return numeric(checked_add(scaled(a, scale), scaled(b, scale)), scale);
}

pli_value pli_neg(pli_value a)
{
    if (a.kind == PLI_CHAR || a.number == INT64_MIN) fail("invalid numeric negation");
    return numeric(-a.number, a.scale);
}

pli_value pli_sub(pli_value a, pli_value b) { return pli_add(a, pli_neg(b)); }

pli_value pli_mul(pli_value a, pli_value b)
{
    if (a.kind == PLI_CHAR || b.kind == PLI_CHAR) fail("invalid multiplication");
    return numeric(checked_mul(a.number, b.number), a.scale + b.scale);
}

pli_value pli_div(pli_value a, pli_value b)
{
    if (a.kind == PLI_CHAR || b.kind == PLI_CHAR || !b.number)
        fail("invalid division");
    if (a.number == INT64_MIN && b.number == -1) fail("numeric overflow");
    if (a.scale < b.scale)
        a.number = checked_mul(a.number, power10(b.scale - a.scale));
    return numeric(a.number / b.number, a.scale > b.scale ? a.scale - b.scale : 0);
}

pli_value pli_concat(pli_value a, pli_value b)
{
    pli_value result;
    if (a.kind != PLI_CHAR) a = pli_char_builtin(a);
    if (b.kind != PLI_CHAR) b = pli_char_builtin(b);
    if (a.length + b.length > PLI_MAX_CHAR) fail("string too long");
    result = pli_storage_char(a.length + b.length);
    memcpy(result.chars, a.chars, a.length);
    memcpy(result.chars + a.length, b.chars, b.length);
    return result;
}

pli_value pli_substr(pli_value a, pli_value start, pli_value length)
{
    pli_value result;
    int64_t pos = scaled(start, 0), n = scaled(length, 0);
    if (a.kind != PLI_CHAR || pos < 1 || n < 0 ||
        (uint64_t)(pos - 1) + (uint64_t)n > a.length)
        fail("SUBSTR out of range");
    result = pli_storage_char((size_t)n);
    memcpy(result.chars, a.chars + pos - 1, (size_t)n);
    return result;
}

pli_value pli_trim(pli_value a)
{
    pli_value result;
    size_t start = 0;
    if (a.kind != PLI_CHAR) fail("TRIM requires a character value");
    while (start < a.length && a.chars[start] == ' ') ++start;
    while (a.length > start && a.chars[a.length - 1] == ' ') --a.length;
    result = pli_storage_char(a.length - start);
    memcpy(result.chars, a.chars + start, a.length - start);
    return result;
}

pli_value pli_centre(pli_value a, pli_value width)
{
    pli_value result;
    int64_t n = scaled(width, 0);
    size_t offset;
    if (a.kind != PLI_CHAR || n < 0 || n > PLI_MAX_CHAR)
        fail("invalid CENTRE width");
    a = pli_trim(a);
    result = pli_storage_char((size_t)n);
    offset = a.length < (size_t)n ? ((size_t)n - a.length) / 2 : 0;
    memcpy(result.chars + offset, a.chars,
           a.length < (size_t)n ? a.length : (size_t)n);
    return result;
}

pli_value pli_char_builtin(pli_value a)
{
    char buffer[80], digits[64];
    int64_t magnitude;
    size_t len, left;
    if (a.kind == PLI_CHAR) return a;
    if (a.number == INT64_MIN) fail("numeric conversion overflow");
    magnitude = a.number < 0 ? -a.number : a.number;
    snprintf(digits, sizeof digits, "%" PRId64, magnitude);
    len = strlen(digits);
    if (a.scale == 0)
        snprintf(buffer, sizeof buffer, "%s%s", a.number < 0 ? "-" : "", digits);
    else {
        left = len > (size_t)a.scale ? len - (size_t)a.scale : 0;
        if (left)
            snprintf(buffer, sizeof buffer, "%s%.*s.%s",
                     a.number < 0 ? "-" : "", (int)left, digits, digits + left);
        else {
            size_t i = 0, zeros = (size_t)a.scale - len;
            if (a.number < 0) buffer[i++] = '-';
            buffer[i++] = '0';
            buffer[i++] = '.';
            while (zeros--) buffer[i++] = '0';
            memcpy(buffer + i, digits, len + 1);
        }
    }
    return pli_literal(buffer);
}

pli_value pli_format_fixed(pli_value a, size_t width, int decimals)
{
    char buffer[80], digits[64];
    pli_value result = pli_storage_char(width);
    int64_t number = scaled(a, decimals);
    uint64_t magnitude;
    size_t length, digit_count;
    if (decimals < 0 || decimals > 17) fail("unsupported F edit scale");
    magnitude = number < 0 ? (uint64_t)(-(number + 1)) + 1 : (uint64_t)number;
    snprintf(digits, sizeof digits, "%" PRIu64, magnitude);
    digit_count = strlen(digits);
    if (decimals == 0) {
        snprintf(buffer, sizeof buffer, "%s%s", number < 0 ? "-" : "", digits);
    } else {
        size_t pos = 0, integer_digits = digit_count > (size_t)decimals ?
                                      digit_count - (size_t)decimals : 0;
        if (number < 0) buffer[pos++] = '-';
        if (integer_digits) {
            memcpy(buffer + pos, digits, integer_digits);
            pos += integer_digits;
        } else {
            buffer[pos++] = '0';
        }
        buffer[pos++] = '.';
        if (!integer_digits && digit_count < (size_t)decimals) {
            size_t zeros = (size_t)decimals - digit_count;
            while (zeros--) buffer[pos++] = '0';
        }
        memcpy(buffer + pos, digits + integer_digits, digit_count - integer_digits);
        pos += digit_count - integer_digits;
        buffer[pos] = '\0';
    }
    length = strlen(buffer);
    if (length > width) {
        memset(result.chars, '*', width);
    } else {
        memcpy(result.chars + width - length, buffer, length);
    }
    return result;
}

pli_value pli_abs(pli_value a)
{
    if (a.kind == PLI_CHAR) fail("ABS requires a number");
    return a.number < 0 ? pli_neg(a) : a;
}

pli_value pli_datetime(void)
{
    char buffer[32];
    time_t now = time(NULL);
    struct tm *clock = localtime(&now);
    if (!clock || !strftime(buffer, sizeof buffer, "%Y%m%d%H%M%S", clock))
        fail("DATETIME failed");
    return pli_literal(buffer);
}

static int compare(pli_value a, pli_value b)
{
    if (a.kind == PLI_CHAR && b.kind == PLI_CHAR) {
        size_t n = a.length > b.length ? a.length : b.length, i;
        for (i = 0; i < n; ++i) {
            unsigned char x = i < a.length ? (unsigned char)a.chars[i] : ' ';
            unsigned char y = i < b.length ? (unsigned char)b.chars[i] : ' ';
            if (x != y) return x < y ? -1 : 1;
        }
        return 0;
    }
    if (a.kind == PLI_CHAR || b.kind == PLI_CHAR) fail("mixed comparison");
    {
        int scale = a.scale > b.scale ? a.scale : b.scale;
        int64_t x = scaled(a, scale), y = scaled(b, scale);
        return x < y ? -1 : x > y ? 1 : 0;
    }
}

pli_value pli_eq(pli_value a, pli_value b) { return pli_bit(compare(a, b) == 0); }
pli_value pli_ne(pli_value a, pli_value b) { return pli_bit(compare(a, b) != 0); }
pli_value pli_lt(pli_value a, pli_value b) { return pli_bit(compare(a, b) < 0); }
pli_value pli_le(pli_value a, pli_value b) { return pli_bit(compare(a, b) <= 0); }
pli_value pli_gt(pli_value a, pli_value b) { return pli_bit(compare(a, b) > 0); }
pli_value pli_ge(pli_value a, pli_value b) { return pli_bit(compare(a, b) >= 0); }
int pli_truth(pli_value a) { return a.kind != PLI_CHAR && a.number != 0; }
pli_value pli_and(pli_value a, pli_value b) { return pli_bit(pli_truth(a) && pli_truth(b)); }
pli_value pli_or(pli_value a, pli_value b) { return pli_bit(pli_truth(a) || pli_truth(b)); }
pli_value pli_not(pli_value a) { return pli_bit(!pli_truth(a)); }

void pli_put_skip_list(size_t count, const pli_value *items)
{
    size_t i, j;
    for (i = 0; i < count; ++i) {
        pli_value item = items[i];
        if (i) putchar(' ');
        if (item.kind != PLI_CHAR) item = pli_char_builtin(item);
        for (j = 0; j < item.length; ++j) putchar(item.chars[j]);
    }
    putchar('\n');
}

void pli_put_page(void) { putchar('\f'); }

void pli_put_skip_edit(pli_value item)
{
    size_t i;
    if (item.kind != PLI_CHAR) item = pli_char_builtin(item);
    for (i = 0; i < item.length; ++i) putchar(item.chars[i]);
    putchar('\n');
}
