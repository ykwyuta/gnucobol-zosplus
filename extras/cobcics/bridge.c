/* Pipe ABI between an isolated GnuCOBOL worker and the Rust TN3270 host.
   The worker must reserve stdout for this binary protocol. */
#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <libcob.h>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#include <process.h>
#include <windows.h>
#else
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>
#endif

#define MAX_FIELDS 64
#define MAX_DATA 256
#define MAX_WIRE_DATA 1024
#define MAX_COMMAREA 32500
#define MAX_TEXT 1919
#define MAX_LINK_DEPTH 8
#define MAX_LOCKS 16
#define MAX_CHANNELS 24
#define MAX_CONTAINERS 12
#define MAX_CONTAINER_DATA 2048
#define MAX_CHILDREN 9

struct field {
    char name[8];
    uint16_t length;
    unsigned char data[MAX_WIRE_DATA];
};

static struct field input_fields[MAX_FIELDS];
static struct field output_fields[MAX_FIELDS];
static unsigned input_count;
static unsigned output_count;
static struct {
    char name[8];
    unsigned char value;
} output_attributes[MAX_FIELDS];
static unsigned output_attribute_count;
static struct {
    char name[8];
    unsigned char kind;
    unsigned char value;
} output_extended_attributes[MAX_FIELDS * 2];
static unsigned output_extended_attribute_count;
static int started;
static FILE *protocol_stdout;
static unsigned char input_commarea[MAX_COMMAREA];
static uint16_t input_commarea_length;
static unsigned char input_aid;
static char input_transid[4];
static char input_applid[8];
static uint16_t link_lengths[MAX_LINK_DEPTH];
static unsigned link_depth;
static char link_callers[MAX_LINK_DEPTH][8];
static void set_cobol_response(unsigned char *value, uint32_t response);
static unsigned screen_root_depth;
static unsigned char output_commarea[MAX_COMMAREA];
static uint16_t output_commarea_length;
static char output_transid[4] = {' ', ' ', ' ', ' '};
static unsigned char screen_action = 4; /* 0: map, 1: erase, 2: data, 3: data+alarm, 4: unchanged. */
static unsigned char return_immediate;
static unsigned char output_text[MAX_TEXT * 4 + 1];
static uint16_t output_text_length;
static uint32_t terminal_uctranst = 451; /* NOUCTRAN */
static unsigned char receive_asis;
struct held_lock {
    unsigned char resource[MAX_DATA];
    uint16_t length;
#ifdef _WIN32
    HANDLE handle;
    OVERLAPPED overlap;
#else
    int fd;
#endif
};
static struct held_lock held_locks[MAX_LOCKS];
static unsigned held_lock_count;
struct container {
    char name[16];
    uint32_t length;
    unsigned char data[MAX_CONTAINER_DATA];
};
struct channel {
    char name[16];
    unsigned scope;
    unsigned count;
    struct container containers[MAX_CONTAINERS];
};
struct child_task {
    char token[16];
    char input_channel[16];
    char reply_channel[16];
    char transid[4];
    unsigned scope;
    int completed;
    int fetched;
};
static struct channel channels[MAX_CHANNELS];
static unsigned channel_count;
static struct child_task children[MAX_CHILDREN];
static unsigned child_count;
static int active_child = -1;
static unsigned current_scope;
static unsigned char saved_aid;
static char saved_transid[4];
static uint16_t saved_commarea_length;
static unsigned saved_input_count;
static unsigned saved_scope;
static struct channel *make_channel(const char *name, unsigned scope);
static struct container *find_container(struct channel *channel,
                                         const char *name);

#ifdef COBCICS_DB2SQL
extern int DB2SQL_END_TASK(int commit);
#endif
#ifdef COBCICS_GENAPP
extern int COBCICS_START_FLUSH(void);
extern void COBCICS_START_CANCEL(void);
#endif

static void failure(const char *message) {
    fprintf(stderr, "cobcics bridge: %s\n", message);
#ifdef COBCICS_GENAPP
    COBCICS_START_CANCEL();
#endif
#ifdef COBCICS_DB2SQL
    if (DB2SQL_END_TASK(0) != 0)
        fprintf(stderr, "cobcics bridge: Db2 rollback failed\n");
#endif
    exit(2);
}

static void read_bytes(void *destination, size_t length) {
    if (fread(destination, 1, length, stdin) != length)
        failure("truncated worker request");
}

static uint16_t read_u16(void) {
    unsigned char bytes[2];
    read_bytes(bytes, 2);
    return (uint16_t)((unsigned)bytes[0] << 8 | bytes[1]);
}

static void write_bytes(const void *source, size_t length) {
    if (!protocol_stdout || fwrite(source, 1, length, protocol_stdout) != length)
        failure("failed to write worker response");
}

static void write_u16(uint16_t number) {
    unsigned char bytes[2] = {(unsigned char)(number >> 8), (unsigned char)number};
    write_bytes(bytes, 2);
}

/* GnuCOBOL COMP halfwords use the z/OS compatible big-endian layout. */
static uint16_t cobol_length(const unsigned char *value) {
    return (uint16_t)((unsigned)value[0] << 8 | value[1]);
}

/* Count encoded COBOL bytes for a UTF-8 worker field. SO/SI delimit a
   two-byte run but do not themselves consume a Unicode character. */
static size_t data_length_for_host(const unsigned char *data, size_t capacity,
                                   const char *host_text) {
    size_t characters = 0, offset = 0;
    int dbcs = 0;
    for (const unsigned char *p = (const unsigned char *)host_text; *p; ++p)
        if ((*p & 0xc0) != 0x80)
            ++characters;
    while (characters && offset < capacity) {
        if (data[offset] == 0x0e) {
            dbcs = 1;
            ++offset;
        } else if (data[offset] == 0x0f) {
            dbcs = 0;
            ++offset;
        } else {
            offset += dbcs ? 2 : 1;
            --characters;
        }
    }
    if (dbcs && offset < capacity && data[offset] == 0x0f)
        ++offset;
    if (characters || offset > capacity)
        failure("cannot determine converted field length");
    return offset;
}

static void set_cobol_length(unsigned char *value, uint16_t length) {
    value[0] = (unsigned char)(length >> 8);
    value[1] = (unsigned char)length;
}

void COBCICS_DEEDIT(unsigned char *field, const unsigned char *length,
                    const unsigned char *capacity) {
    uint16_t count = cobol_length(length);
    if (!started || count == 0 || count > cobol_length(capacity))
        failure("invalid BIF DEEDIT field length");
    if (count == 1)
        return;
    size_t write = count;
    for (size_t read = count; read > 0; --read) {
        unsigned char digit = field[read - 1];
        if (digit >= '0' && digit <= '9')
            field[--write] = digit;
    }
    memset(field, '0', write);
}

void COBCICS_START(unsigned char *aid, unsigned char *calen,
                   unsigned char *transid) {
    (void)transid; /* Third argument is accessed through libcob's call ABI. */
    unsigned char header[5];
    int has_containers;
    if (active_child >= 0) {
        char host_transid[5];
        *aid = 0;
        set_cobol_length(calen, 0);
        memcpy(host_transid, input_transid, 4);
        host_transid[4] = 0;
        if (cob_put_param_str(3, host_transid) != 0)
            failure("cannot convert child TRANSID to COBOL data");
        return;
    }
    if (started)
        failure("worker request read twice");
#ifdef _WIN32
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
#endif
    /* COBOL DISPLAY must not corrupt the binary response on stdout. Keep a
       separate handle for the protocol and send ordinary DISPLAY to stderr. */
    if (fflush(stdout) != 0)
        failure("failed to flush worker stdout");
#ifdef _WIN32
    int protocol_fd = _dup(_fileno(stdout));
    if (protocol_fd < 0 || _dup2(_fileno(stderr), _fileno(stdout)) != 0)
        failure("failed to separate worker protocol output");
    protocol_stdout = _fdopen(protocol_fd, "wb");
#else
    int protocol_fd = dup(fileno(stdout));
    if (protocol_fd < 0 || dup2(fileno(stderr), fileno(stdout)) < 0)
        failure("failed to separate worker protocol output");
    protocol_stdout = fdopen(protocol_fd, "wb");
#endif
    if (!protocol_stdout)
        failure("failed to open worker protocol output");
    read_bytes(header, sizeof header);
    has_containers = memcmp(header, "CCI4", 4) == 0;
    if (!has_containers && memcmp(header, "CCI3", 4) != 0)
        failure("unsupported worker request version");
    *aid = header[4];
    input_aid = header[4];
    read_bytes(input_transid, 4);
    {
        char host_transid[5];
        memcpy(host_transid, input_transid, 4);
        host_transid[4] = 0;
        if (cob_put_param_str(3, host_transid) != 0)
            failure("cannot convert TRANSID to COBOL data");
    }
    read_bytes(input_applid, 8);
    input_commarea_length = read_u16();
    if (input_commarea_length > MAX_COMMAREA)
        failure("input COMMAREA exceeds 32500 bytes");
    read_bytes(input_commarea, input_commarea_length);
    set_cobol_length(calen, input_commarea_length);
    input_count = read_u16();
    if (input_count > MAX_FIELDS)
        failure("too many input fields");
    for (unsigned index = 0; index < input_count; ++index) {
        struct field *field = &input_fields[index];
        read_bytes(field->name, 8);
        field->length = read_u16();
        if (field->length > MAX_WIRE_DATA)
            failure("input field exceeds UTF-8 worker limit");
        read_bytes(field->data, field->length);
    }
    if (has_containers) {
        unsigned count = read_u16();
        if (count > MAX_CHANNELS * MAX_CONTAINERS)
            failure("too many input containers");
        for (unsigned index = 0; index < count; ++index) {
            char channel_name[16], container_name[16];
            read_bytes(channel_name, sizeof channel_name);
            read_bytes(container_name, sizeof container_name);
            unsigned length = read_u16();
            if (length > MAX_CONTAINER_DATA)
                failure("input container exceeds 2048 bytes");
            struct channel *channel = make_channel(channel_name, current_scope);
            if (!channel || find_container(channel, container_name) ||
                channel->count >= MAX_CONTAINERS)
                failure("duplicate input container or channel limit");
            struct container *container = &channel->containers[channel->count++];
            memcpy(container->name, container_name, 16);
            container->length = length;
            read_bytes(container->data, length);
        }
    }
    started = 1;
}

void COBCICS_LINK_ENTER(const unsigned char *length,
                        const unsigned char *capacity) {
    uint16_t count = cobol_length(length);
    uint16_t available = cobol_length(capacity);
    if (!started || link_depth >= MAX_LINK_DEPTH ||
        count > 32767 || count > available)
        failure("invalid LINK COMMAREA length or nesting depth");
    link_lengths[link_depth++] = count;
    memset(link_callers[link_depth - 1], ' ', 8);
}

void COBCICS_LINK_CALLER(const char *program) {
    char host_program[9];
    if (!started || link_depth == 0)
        failure("LINK_CALLER outside LINK");
    (void)program;
    if (!cob_get_param_str(1, host_program, sizeof host_program))
        failure("cannot convert LINK caller name");
    memcpy(link_callers[link_depth - 1], host_program, 8);
}

void COBCICS_SCREEN_ENTER(const unsigned char *length,
                          const unsigned char *capacity) {
    if (link_depth != 0 || screen_root_depth != 0)
        failure("SCREEN_ENTER requires a top-level task");
    COBCICS_LINK_ENTER(length, capacity);
    screen_root_depth = 1;
}

void COBCICS_LINK_START(unsigned char *aid, unsigned char *calen,
                        unsigned char *transid) {
    char host_transid[5];
    if (!started)
        failure("LINK_START before worker request");
    *aid = input_aid;
    set_cobol_length(calen, link_depth ? link_lengths[link_depth - 1]
                                       : input_commarea_length);
    memcpy(host_transid, input_transid, 4);
    host_transid[4] = 0;
    (void)transid;
    if (cob_put_param_str(3, host_transid) != 0)
        failure("cannot convert LINK TRANSID to COBOL data");
}

void COBCICS_LINK_EXIT(void) {
    if (!started || link_depth == 0)
        failure("LINK_EXIT without LINK_ENTER");
    --link_depth;
}

void COBCICS_TASK_NUMBER(unsigned char *destination) {
    if (!started)
        failure("TASK_NUMBER before START");
#ifdef _WIN32
    unsigned number = (unsigned)_getpid() % 10000000U;
#else
    unsigned number = (unsigned)getpid() % 10000000U;
#endif
    if (active_child >= 0)
        number = (number + (unsigned)active_child + 1) % 10000000U;
    char digits[8];
    if (snprintf(digits, sizeof digits, "%07u", number) != 7)
        failure("invalid synthetic task number");
    for (unsigned index = 0; index < 3; ++index)
        destination[index] = (unsigned char)(((digits[index * 2] - '0') << 4) |
                                             (digits[index * 2 + 1] - '0'));
    destination[3] = (unsigned char)(((digits[6] - '0') << 4) | 0x0c);
}

void COBCICS_GET_COMMAREA(unsigned char *destination,
                           unsigned char *capacity_and_length) {
    uint16_t capacity = cobol_length(capacity_and_length);
    if (!started || capacity > MAX_COMMAREA || input_commarea_length > capacity)
        failure("invalid GET_COMMAREA call");
    memset(destination, 0, capacity);
    memcpy(destination, input_commarea, input_commarea_length);
    set_cobol_length(capacity_and_length, input_commarea_length);
}

void COBCICS_RECEIVE_INTO(unsigned char *destination,
                          unsigned char *length,
                          const unsigned char *capacity,
                          unsigned char *resp, unsigned char *resp2) {
    uint16_t available = cobol_length(length);
    uint16_t size = cobol_length(capacity);
    if (!started)
        failure("RECEIVE INTO before worker request");
    if (available > size)
        available = size;
    uint16_t host_count = 5;
    if (input_count)
        host_count += input_fields[0].length;
    {
        char host_text[MAX_WIRE_DATA + 6];
        memset(host_text, ' ', host_count);
        memcpy(host_text, input_transid, 4);
        if (input_count)
            memcpy(host_text + 5, input_fields[0].data,
                   input_fields[0].length);
        host_text[host_count] = 0;
        (void)destination;
        if (cob_put_param_str(1, host_text) != 0) {
            set_cobol_response(resp, 22); /* LENGERR */
            set_cobol_response(resp2, 0);
            return;
        }
        size_t count = data_length_for_host(destination, size, host_text);
        set_cobol_length(length, (uint16_t)count);
        if (count > available) {
            set_cobol_response(resp, 22);
            set_cobol_response(resp2, 0);
            return;
        }
    }
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_ASSIGN_APPLID(unsigned char *destination,
                           const unsigned char *length) {
    if (!started || cobol_length(length) != 8)
        failure("ASSIGN APPLID requires an 8-byte area");
    char host_applid[9];
    memcpy(host_applid, input_applid, 8);
    host_applid[8] = 0;
    (void)destination;
    if (cob_put_param_str(1, host_applid) != 0)
        failure("cannot convert ASSIGN APPLID");
}

void COBCICS_ASSIGN_PROGRAM(unsigned char *destination,
                            const unsigned char *length, const char *program) {
    if (!started || cobol_length(length) != 8)
        failure("ASSIGN PROGRAM requires an 8-byte area");
    (void)destination;
    (void)program;
    char host_program[9];
    if (!cob_get_param_str(3, host_program, sizeof host_program) ||
        cob_put_param_str(1, host_program) != 0)
        failure("cannot convert ASSIGN PROGRAM");
}

void COBCICS_ASSIGN_SYSID(unsigned char *destination,
                          const unsigned char *length) {
    if (!started || cobol_length(length) != 4)
        failure("ASSIGN SYSID requires a 4-byte area");
    (void)destination;
    if (cob_put_param_str(1, "L001") != 0)
        failure("cannot convert ASSIGN SYSID");
}

void COBCICS_ASSIGN_STARTCODE(unsigned char *destination,
                              const unsigned char *length) {
    if (!started || cobol_length(length) != 2)
        failure("ASSIGN STARTCODE requires a 2-byte area");
    (void)destination;
    if (cob_put_param_str(1, link_depth ? "DL" : "TO") != 0)
        failure("cannot convert ASSIGN STARTCODE");
}

void COBCICS_ASSIGN_INVOKINGPROG(unsigned char *destination,
                                 const unsigned char *length) {
    if (!started || cobol_length(length) != 8)
        failure("ASSIGN INVOKINGPROG requires an 8-byte area");
    char host_program[9];
    if (link_depth)
        memcpy(host_program, link_callers[link_depth - 1], 8);
    else
        memset(host_program, ' ', 8);
    host_program[8] = 0;
    (void)destination;
    if (cob_put_param_str(1, host_program) != 0)
        failure("cannot convert ASSIGN INVOKINGPROG");
}

void COBCICS_GET_FIELD(const char *name, unsigned char *destination,
                       unsigned char *capacity_and_length, unsigned char *flag) {
    uint16_t capacity = cobol_length(capacity_and_length);
    char host_name[9];
    if (!started || capacity > MAX_DATA)
        failure("invalid GET_FIELD call");
    (void)name;
    if (!cob_get_param_str(1, host_name, sizeof host_name))
        failure("cannot convert input field name");
    memset(destination, 0, (size_t)capacity);
    set_cobol_length(capacity_and_length, 0);
    *flag = 0;
    for (unsigned index = 0; index < input_count; ++index) {
        const struct field *field = &input_fields[index];
        if (memcmp(field->name, host_name, 8) != 0)
            continue;
        size_t data_length;
        {
            char host_data[MAX_WIRE_DATA + 1];
            memcpy(host_data, field->data, field->length);
            host_data[field->length] = 0;
            if (terminal_uctranst == 450 && !receive_asis)
                for (unsigned offset = 0; offset < field->length; ++offset)
                    if (host_data[offset] >= 'a' && host_data[offset] <= 'z')
                        host_data[offset] = (char)(host_data[offset] - 'a' + 'A');
            if (cob_put_param_str(2, host_data) != 0)
                failure("cannot convert input field data");
            data_length = data_length_for_host(destination, capacity, host_data);
        }
        set_cobol_length(capacity_and_length, (uint16_t)data_length);
        *flag = data_length == 0 ? 0x80 : 0;
        return;
    }
}

static void set_cobol_response(unsigned char *value, uint32_t response) {
    value[0] = (unsigned char)(response >> 24);
    value[1] = (unsigned char)(response >> 16);
    value[2] = (unsigned char)(response >> 8);
    value[3] = (unsigned char)response;
}

static uint32_t cobol_fullword(const unsigned char *value) {
    return (uint32_t)value[0] << 24 | (uint32_t)value[1] << 16 |
           (uint32_t)value[2] << 8 | value[3];
}

void COBCICS_INQUIRE_TERMINAL(const char *termid, unsigned char *uctranst,
                              unsigned char *resp, unsigned char *resp2) {
    if (!started || memcmp(termid, "L001", 4) != 0)
        failure("invalid INQUIRE TERMINAL call");
    set_cobol_response(uctranst, terminal_uctranst);
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_SET_TERMINAL(const char *termid, const unsigned char *uctranst,
                          unsigned char *resp, unsigned char *resp2) {
    if (!started || memcmp(termid, "L001", 4) != 0)
        failure("invalid SET TERMINAL call");
    uint32_t value = cobol_fullword(uctranst);
    if (value != 450 && value != 451 && value != 452) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    terminal_uctranst = value;
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_SET_RECEIVE_ASIS(const unsigned char *mode) {
    if (!started || (*mode != 0 && *mode != 1))
        failure("invalid RECEIVE ASIS mode");
    receive_asis = *mode;
}

void COBCICS_INQUIRE_ASSOCIATION(unsigned char *applid,
                                 unsigned char *userid,
                                 unsigned char *facility,
                                 unsigned char *networkid,
                                 unsigned char *facility_type) {
    if (!started)
        failure("INQUIRE ASSOCIATION before START");
    memcpy(applid, input_applid, 8);
    memcpy(userid, "COBCICS ", 8);
    memcpy(facility, "L001    ", 8);
    memcpy(networkid, "LOCAL   ", 8);
    set_cobol_response(facility_type, 213); /* TERMINAL */
}

static int find_lock(const unsigned char *resource, uint16_t length) {
    for (unsigned index = 0; index < held_lock_count; ++index)
        if (held_locks[index].length == length &&
            memcmp(held_locks[index].resource, resource, length) == 0)
            return (int)index;
    return -1;
}

static int lock_path(const unsigned char *resource, uint16_t length,
                     char *path, size_t capacity) {
    uint64_t hash = 14695981039346656037ULL;
    for (uint16_t index = 0; index < length; ++index)
        hash = (hash ^ resource[index]) * 1099511628211ULL;
    const char *directory = getenv("COBCICS_LOCK_DIR");
    if (!directory || !*directory) {
#ifdef _WIN32
        directory = getenv("TEMP");
        if (!directory || !*directory)
            directory = ".";
#else
        directory = "/tmp";
#endif
    }
    int written = snprintf(path, capacity, "%s/cobcics-enq-%016llx.lock",
                           directory, (unsigned long long)hash);
    return written > 0 && (size_t)written < capacity;
}

void COBCICS_ENQ(const unsigned char *resource, const unsigned char *length,
                 const unsigned char *capacity, unsigned char *resp,
                 unsigned char *resp2) {
    uint16_t count = cobol_length(length);
    if (!started || count == 0 || count > MAX_DATA ||
        count > cobol_length(capacity))
        failure("invalid ENQ resource length");
    if (held_lock_count >= MAX_LOCKS || find_lock(resource, count) >= 0) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    char path[512];
    if (!lock_path(resource, count, path, sizeof path))
        failure("ENQ lock path is too long");
    struct held_lock *entry = &held_locks[held_lock_count];
#ifdef _WIN32
    entry->handle = CreateFileA(path, GENERIC_READ | GENERIC_WRITE,
                                FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
                                OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (entry->handle == INVALID_HANDLE_VALUE) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 2);
        return;
    }
    memset(&entry->overlap, 0, sizeof entry->overlap);
    if (!LockFileEx(entry->handle, LOCKFILE_EXCLUSIVE_LOCK, 0,
                    1, 0, &entry->overlap)) {
        CloseHandle(entry->handle);
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 3);
        return;
    }
#else
    entry->fd = open(path, O_CREAT | O_RDWR, 0600);
    if (entry->fd < 0 || flock(entry->fd, LOCK_EX) != 0) {
        if (entry->fd >= 0)
            close(entry->fd);
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 2);
        return;
    }
#endif
    memcpy(entry->resource, resource, count);
    entry->length = count;
    ++held_lock_count;
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_DEQ(const unsigned char *resource, const unsigned char *length,
                 const unsigned char *capacity, unsigned char *resp,
                 unsigned char *resp2) {
    uint16_t count = cobol_length(length);
    if (!started || count == 0 || count > MAX_DATA ||
        count > cobol_length(capacity))
        failure("invalid DEQ resource length");
    int index = find_lock(resource, count);
    if (index < 0) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    struct held_lock *entry = &held_locks[index];
#ifdef _WIN32
    int ok = UnlockFileEx(entry->handle, 0, 1, 0, &entry->overlap);
    CloseHandle(entry->handle);
#else
    int ok = flock(entry->fd, LOCK_UN) == 0;
    close(entry->fd);
#endif
    held_locks[index] = held_locks[--held_lock_count];
    set_cobol_response(resp, ok ? 0 : 16);
    set_cobol_response(resp2, ok ? 0 : 2);
}

static struct channel *find_channel(const char *name, unsigned scope) {
    for (unsigned index = 0; index < channel_count; ++index)
        if (channels[index].scope == scope &&
            memcmp(channels[index].name, name, 16) == 0)
            return &channels[index];
    return NULL;
}

static struct channel *make_channel(const char *name, unsigned scope) {
    struct channel *result = find_channel(name, scope);
    if (result)
        return result;
    if (channel_count >= MAX_CHANNELS)
        return NULL;
    result = &channels[channel_count++];
    memset(result, 0, sizeof *result);
    memcpy(result->name, name, 16);
    result->scope = scope;
    return result;
}

static struct container *find_container(struct channel *channel,
                                         const char *name) {
    for (unsigned index = 0; index < channel->count; ++index)
        if (memcmp(channel->containers[index].name, name, 16) == 0)
            return &channel->containers[index];
    return NULL;
}

void COBCICS_PUT_CONTAINER(const char *name, const unsigned char *source,
                           const unsigned char *length,
                           const unsigned char *capacity, const char *channel_name,
                           unsigned char *resp, unsigned char *resp2) {
    uint32_t count = cobol_fullword(length);
    if (!started || count > MAX_CONTAINER_DATA ||
        count > cobol_fullword(capacity)) {
        set_cobol_response(resp, 22); /* LENGERR */
        set_cobol_response(resp2, 1);
        return;
    }
    struct channel *channel = make_channel(channel_name, current_scope);
    struct container *container = channel ? find_container(channel, name) : NULL;
    if (channel && !container && channel->count < MAX_CONTAINERS) {
        container = &channel->containers[channel->count++];
        memcpy(container->name, name, 16);
    }
    if (!container) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    memcpy(container->data, source, count);
    container->length = count;
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_GET_CONTAINER(const char *name, unsigned char *destination,
                           unsigned char *length, const unsigned char *capacity,
                           const char *channel_name, unsigned char *resp,
                           unsigned char *resp2) {
    if (!started)
        failure("GET CONTAINER before START");
    struct channel *channel = find_channel(channel_name, current_scope);
    struct container *container = channel ? find_container(channel, name) : NULL;
    if (!container) {
        set_cobol_response(resp, 13); /* NOTFND */
        set_cobol_response(resp2, 1);
        return;
    }
    uint32_t available = cobol_fullword(length);
    uint32_t size = cobol_fullword(capacity);
    if (container->length > available || container->length > size) {
        fprintf(stderr, "cobcics GET CONTAINER: stored=%u requested=%u capacity=%u\n",
                container->length, available, size);
        set_cobol_response(length, container->length);
        set_cobol_response(resp, 22);
        set_cobol_response(resp2, 1);
        return;
    }
    memcpy(destination, container->data, container->length);
    set_cobol_response(length, container->length);
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_RUN_ENTER(const char *transid, const char *channel_name,
                       char *token, char *program, unsigned char *resp,
                       unsigned char *resp2) {
    if (!started || active_child >= 0)
        failure("invalid RUN task nesting");
    if (child_count >= MAX_CHILDREN || memcmp(transid, "OCR", 3) != 0 ||
        transid[3] < '1' || transid[3] > '5') {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    struct channel *source = make_channel(channel_name, current_scope);
    unsigned scope = child_count + 1;
    struct channel *child_channel = make_channel(channel_name, scope);
    if (!source || !child_channel) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 2);
        return;
    }
    child_channel->count = source->count;
    memcpy(child_channel->containers, source->containers,
           sizeof source->containers);
    struct child_task *child = &children[child_count];
    char value[17];
    if (snprintf(value, sizeof value, "%016u", child_count + 1) != 16)
        failure("cannot create child token");
    memcpy(child->token, value, 16);
    memcpy(child->input_channel, channel_name, 16);
    if (snprintf(value, sizeof value, "CCRC%012u", child_count + 1) != 16)
        failure("cannot create child channel");
    memcpy(child->reply_channel, value, 16);
    memcpy(child->transid, transid, 4);
    child->scope = scope;
    child->completed = 0;
    child->fetched = 0;
    memcpy(token, child->token, 16);
    memcpy(program, "CRDTAGY1", 8);
    program[7] = transid[3];
    saved_aid = input_aid;
    memcpy(saved_transid, input_transid, 4);
    saved_commarea_length = input_commarea_length;
    saved_input_count = input_count;
    saved_scope = current_scope;
    input_aid = 0;
    memcpy(input_transid, transid, 4);
    input_commarea_length = 0;
    input_count = 0;
    current_scope = scope;
    active_child = (int)child_count++;
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_RUN_EXIT(void) {
    if (!started || active_child < 0)
        failure("RUN_EXIT without child");
    struct child_task *child = &children[active_child];
    struct channel *result = find_channel(child->input_channel, child->scope);
    struct channel *reply = make_channel(child->reply_channel, saved_scope);
    if (!reply)
        failure("too many reply channels");
    if (result) {
        reply->count = result->count;
        memcpy(reply->containers, result->containers,
               sizeof result->containers);
    }
    child->completed = 1;
    current_scope = saved_scope;
    input_aid = saved_aid;
    memcpy(input_transid, saved_transid, 4);
    input_commarea_length = saved_commarea_length;
    input_count = saved_input_count;
    active_child = -1;
}

void COBCICS_FETCH_ANY(char *token, char *channel_name,
                       unsigned char *status, char *abcode,
                       unsigned char *resp, unsigned char *resp2) {
    if (!started || active_child >= 0)
        failure("invalid FETCH ANY call");
    if (child_count == 0) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 52);
        return;
    }
    for (unsigned index = 0; index < child_count; ++index) {
        struct child_task *child = &children[index];
        if (child->fetched || !child->completed)
            continue;
        memcpy(token, child->token, 16);
        memcpy(channel_name, child->reply_channel, 16);
        set_cobol_response(status, 1016); /* NORMAL */
        memset(abcode, ' ', 4);
        set_cobol_response(resp, 0);
        set_cobol_response(resp2, 0);
        child->fetched = 1;
        return;
    }
    set_cobol_response(resp, 13); /* NOTFND */
    set_cobol_response(resp2, 1);
}

void COBCICS_WRITE_ABNDFILE(const unsigned char *record,
                            const unsigned char *record_length,
                            const unsigned char *key,
                            const unsigned char *key_length,
                            unsigned char *resp, unsigned char *resp2) {
    uint16_t count = cobol_length(record_length);
    uint16_t key_count = cobol_length(key_length);
    if (!started || count == 0 || count > 32767 || key_count != 12 ||
        key_count > count || memcmp(record, key, key_count) != 0)
        failure("invalid ABNDFILE WRITE record or key");
    const char *path = getenv("COBCICS_ABEND_LOG");
    if (!path || !*path) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    FILE *file = fopen(path, "ab");
    unsigned char header[2] = {(unsigned char)(count >> 8),
                               (unsigned char)count};
    int ok = file && fwrite(header, 1, 2, file) == 2 &&
        fwrite(record, 1, count, file) == count;
    if (file && fclose(file) != 0)
        ok = 0;
    set_cobol_response(resp, ok ? 0 : 16);
    set_cobol_response(resp2, ok ? 0 : 2);
}

void COBCICS_SYNCPOINT(const unsigned char *mode, unsigned char *resp,
                       unsigned char *resp2) {
    if (!started || (*mode != 0 && *mode != 1))
        failure("invalid SYNCPOINT call");
#ifdef COBCICS_DB2SQL
    if (DB2SQL_END_TASK(*mode == 0) != 0)
        failure("Db2 SYNCPOINT failed");
#endif
#ifdef COBCICS_GENAPP
    if (*mode == 0) {
        if (COBCICS_START_FLUSH() != 0)
            failure("CICS START scheduling failed");
    } else {
        COBCICS_START_CANCEL();
    }
#endif
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_RETRIEVE(unsigned char *destination,
                      const unsigned char *capacity,
                      unsigned char *resp, unsigned char *resp2) {
    const char *queue = getenv("COBCICS_MQ_TRIGGER_QUEUE");
    size_t length;
    if (!started)
        failure("invalid RETRIEVE call");
    if (!queue || !*queue) {
        set_cobol_response(resp, 44); /* QIDERR: no trigger data */
        set_cobol_response(resp2, 1);
        return;
    }
    length = strlen(queue);
    if (length > 48 || cobol_fullword(capacity) < 112) {
        set_cobol_response(resp, 22); /* LENGERR */
        set_cobol_response(resp2, 0);
        return;
    }
    memset(destination, ' ', 112);
    memcpy(destination, queue, length);
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_DELAY(const unsigned char *seconds) {
    uint16_t count = cobol_length(seconds);
    if (!started || count > 60)
        failure("invalid DELAY seconds");
#ifdef _WIN32
    Sleep((DWORD)count * 1000);
#else
    sleep(count);
#endif
}

static int64_t days_from_civil(int year, unsigned month, unsigned day) {
    year -= month <= 2;
    int era = (year >= 0 ? year : year - 399) / 400;
    unsigned yoe = (unsigned)(year - era * 400);
    unsigned shifted_month = (unsigned)((int)month + (month > 2 ? -3 : 9));
    unsigned doy = (153 * shifted_month + 2) / 5 + day - 1;
    unsigned doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return (int64_t)era * 146097 + doe - 719468;
}

static void civil_from_days(int64_t days, int *year, unsigned *month,
                            unsigned *day) {
    int64_t z = days + 719468;
    int64_t era = (z >= 0 ? z : z - 146096) / 146097;
    unsigned doe = (unsigned)(z - era * 146097);
    unsigned yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int y = (int)yoe + (int)era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    unsigned mp = (5 * doy + 2) / 153;
    *day = doy - (153 * mp + 2) / 5 + 1;
    *month = (unsigned)((int)mp + (mp < 10 ? 3 : -9));
    *year = y + (*month <= 2);
}

/* The Bank sample calls these Language Environment date routines. The
   day origin is shared by both calls, which is what its age check needs. */
void COBCICS_CEEDAYS(const unsigned char *date, const unsigned char *format,
                     unsigned char *day_number, unsigned char *condition) {
    memset(condition, 0, 12);
    if (cobol_length(date) != 10 || cobol_length(format) != 10 ||
        memcmp(format + 2, "YYYYMMDD", 8) != 0) {
        condition[0] = 1;
        return;
    }
    int year = 0, month = 0, day = 0;
    for (int index = 2; index < 10; ++index) {
        if (date[index] < '0' || date[index] > '9') {
            condition[0] = 1;
            return;
        }
        int digit = date[index] - '0';
        if (index < 6) year = year * 10 + digit;
        else if (index < 8) month = month * 10 + digit;
        else day = day * 10 + digit;
    }
    if (year < 1 || month < 1 || month > 12 || day < 1 || day > 31) {
        condition[0] = 1;
        return;
    }
    int64_t days = days_from_civil(year, (unsigned)month, (unsigned)day);
    int checked_year;
    unsigned checked_month, checked_day;
    civil_from_days(days, &checked_year, &checked_month, &checked_day);
    if (checked_year != year || checked_month != (unsigned)month ||
        checked_day != (unsigned)day) {
        condition[0] = 1;
        return;
    }
    set_cobol_response(day_number,
        (uint32_t)(days - days_from_civil(1582, 10, 14)));
}

void COBCICS_CEELOCT(unsigned char *day_number, unsigned char *seconds,
                     unsigned char *gregorian, unsigned char *condition) {
    memset(condition, 0, 12);
    time_t now = time(NULL);
    struct tm *local = localtime(&now);
    if (!local) {
        condition[0] = 1;
        return;
    }
    set_cobol_response(day_number, (uint32_t)(days_from_civil(
        local->tm_year + 1900, (unsigned)local->tm_mon + 1,
        (unsigned)local->tm_mday) - days_from_civil(1582, 10, 14)));
    memset(seconds, 0, 8);
    char formatted[9];
    snprintf(formatted, sizeof formatted, "%04d%02d%02d",
             local->tm_year + 1900, local->tm_mon + 1, local->tm_mday);
    memcpy(gregorian, formatted, 8);
}

static void pack_abstime(unsigned char *destination, uint64_t value) {
    char digits[16];
    if (value >= 1000000000000000ULL ||
        snprintf(digits, sizeof digits, "%015llu", (unsigned long long)value) != 15)
        failure("ASKTIME result is out of range");
    for (unsigned index = 0; index < 7; ++index)
        destination[index] = (unsigned char)(((digits[index * 2] - '0') << 4) |
                                             (digits[index * 2 + 1] - '0'));
    destination[7] = (unsigned char)(((digits[14] - '0') << 4) | 0x0c);
}

static int unpack_abstime(const unsigned char *source, uint64_t *value) {
    uint64_t result = 0;
    for (unsigned index = 0; index < 15; ++index) {
        unsigned digit = index & 1 ? source[index / 2] & 0x0f
                                   : source[index / 2] >> 4;
        if (digit > 9)
            return 0;
        result = result * 10 + digit;
    }
    if ((source[7] & 0x0f) != 0x0c && (source[7] & 0x0f) != 0x0f)
        return 0;
    *value = result;
    return 1;
}

void COBCICS_ASKTIME(unsigned char *abstime, const unsigned char *capacity) {
    struct timespec now;
    if (!started || cobol_length(capacity) != 8 ||
        timespec_get(&now, TIME_UTC) != TIME_UTC)
        failure("invalid ASKTIME call or system clock");
    struct tm *local = localtime(&now.tv_sec);
    if (!local)
        failure("cannot read local time");
    int64_t days = days_from_civil(local->tm_year + 1900,
                                   (unsigned)local->tm_mon + 1,
                                   (unsigned)local->tm_mday) + 25567;
    if (days < 0)
        failure("ASKTIME is before 1900");
    uint64_t milliseconds = (uint64_t)days * 86400000ULL +
        (uint64_t)(local->tm_hour * 3600 + local->tm_min * 60 + local->tm_sec) * 1000ULL +
        (uint64_t)(now.tv_nsec / 1000000);
    pack_abstime(abstime, milliseconds);
}

static void format_time(const unsigned char *abstime,
                         const unsigned char *abstime_capacity,
                         unsigned char *date, const unsigned char *date_capacity,
                         unsigned char *clock, const unsigned char *clock_capacity,
                         const unsigned char *separator,
                         unsigned char *resp, unsigned char *resp2,
                         int month_first, unsigned char time_separator) {
    if (!started || cobol_length(abstime_capacity) != 8 ||
        cobol_length(date_capacity) < 10 ||
        cobol_length(clock_capacity) < (time_separator ? 8 : 6))
        failure("invalid FORMATTIME data area length");
    uint64_t milliseconds;
    if (!unpack_abstime(abstime, &milliseconds)) {
        set_cobol_response(resp, 16); /* INVREQ */
        set_cobol_response(resp2, 1);
        return;
    }
    uint64_t days = milliseconds / 86400000ULL;
    uint64_t seconds = milliseconds % 86400000ULL / 1000ULL;
    int year;
    unsigned month, day;
    civil_from_days((int64_t)days - 25567, &year, &month, &day);
    if (year < 1900 || year > 9999 ||
        (*separator != 0 && (*separator < 0x20 || *separator > 0x7e))) {
        set_cobol_response(resp, 16);
        set_cobol_response(resp2, 1);
        return;
    }
    char formatted_date[11];
    char formatted_time[9];
    int date_size;
    if (*separator && month_first)
        date_size = snprintf(formatted_date, sizeof formatted_date,
                             "%02u%c%02u%c%04d", month, *separator,
                             day, *separator, year);
    else if (*separator)
        date_size = snprintf(formatted_date, sizeof formatted_date,
                             "%02u%c%02u%c%04d", day, *separator,
                             month, *separator, year);
    else if (month_first)
        date_size = snprintf(formatted_date, sizeof formatted_date,
                             "%02u%02u%04d", month, day, year);
    else
        date_size = snprintf(formatted_date, sizeof formatted_date,
                             "%02u%02u%04d", day, month, year);
    int time_size;
    if (time_separator)
        time_size = snprintf(formatted_time, sizeof formatted_time,
                             "%02llu%c%02llu%c%02llu",
                             (unsigned long long)(seconds / 3600), time_separator,
                             (unsigned long long)(seconds / 60 % 60), time_separator,
                             (unsigned long long)(seconds % 60));
    else
        time_size = snprintf(formatted_time, sizeof formatted_time,
                             "%02llu%02llu%02llu",
                             (unsigned long long)(seconds / 3600),
                             (unsigned long long)(seconds / 60 % 60),
                             (unsigned long long)(seconds % 60));
    if (date_size < 0 || date_size > 10 || time_size != (time_separator ? 8 : 6))
        failure("FORMATTIME formatting failed");
    memset(date, ' ', cobol_length(date_capacity));
    memcpy(date, formatted_date, (size_t)date_size);
    memset(clock, ' ', cobol_length(clock_capacity));
    memcpy(clock, formatted_time, (size_t)time_size);
    set_cobol_response(resp, 0);
    set_cobol_response(resp2, 0);
}

void COBCICS_FORMATTIME(const unsigned char *abstime,
                         const unsigned char *abstime_capacity,
                         unsigned char *date, const unsigned char *date_capacity,
                         unsigned char *clock, const unsigned char *clock_capacity,
                         const unsigned char *separator,
                         unsigned char *resp, unsigned char *resp2) {
    format_time(abstime, abstime_capacity, date, date_capacity,
                clock, clock_capacity, separator, resp, resp2, 0, 0);
}

void COBCICS_FORMATTIME_MMDD(const unsigned char *abstime,
                              const unsigned char *abstime_capacity,
                              unsigned char *date, const unsigned char *date_capacity,
                              unsigned char *clock, const unsigned char *clock_capacity,
                              const unsigned char *separator,
                              unsigned char *resp, unsigned char *resp2) {
    format_time(abstime, abstime_capacity, date, date_capacity,
                clock, clock_capacity, separator, resp, resp2, 1, 0);
}

void COBCICS_FORMATTIME_TIMESEP(const unsigned char *abstime,
                         const unsigned char *abstime_capacity,
                         unsigned char *date, const unsigned char *date_capacity,
                         unsigned char *clock, const unsigned char *clock_capacity,
                         const unsigned char *separator,
                         const unsigned char *time_separator,
                         unsigned char *resp, unsigned char *resp2) {
    format_time(abstime, abstime_capacity, date, date_capacity,
                clock, clock_capacity, separator, resp, resp2, 0,
                *time_separator);
}

void COBCICS_FORMATTIME_MMDD_TIMESEP(const unsigned char *abstime,
                         const unsigned char *abstime_capacity,
                         unsigned char *date, const unsigned char *date_capacity,
                         unsigned char *clock, const unsigned char *clock_capacity,
                         const unsigned char *separator,
                         const unsigned char *time_separator,
                         unsigned char *resp, unsigned char *resp2) {
    format_time(abstime, abstime_capacity, date, date_capacity,
                clock, clock_capacity, separator, resp, resp2, 1,
                *time_separator);
}

void COBCICS_ABEND(const char *abcode, const unsigned char *length) {
    uint16_t count = cobol_length(length);
    if (!started || count == 0 || count > 4)
        failure("invalid ABEND code length");
    char code[5] = {' ', ' ', ' ', ' ', '\0'};
    memcpy(code, abcode, count);
    for (unsigned index = 0; index < count; ++index)
        if (!((code[index] >= 'A' && code[index] <= 'Z') ||
              (code[index] >= '0' && code[index] <= '9') ||
              code[index] == ' '))
            failure("invalid ABEND code");
    fprintf(stderr, "cobcics abend: %.4s\n", code);
#ifdef COBCICS_GENAPP
    COBCICS_START_CANCEL();
#endif
#ifdef COBCICS_DB2SQL
    if (DB2SQL_END_TASK(0) != 0)
        fprintf(stderr, "cobcics bridge: Db2 rollback failed\n");
#endif
    exit(12);
}

void COBCICS_REQUIRE_NORMAL(const unsigned char *resp) {
    if (!started)
        failure("REQUIRE_NORMAL before START");
    if (resp[0] != 0 || resp[1] != 0 || resp[2] != 0 || resp[3] != 0) {
        uint32_t code = (uint32_t)resp[0] << 24 | (uint32_t)resp[1] << 16 |
                        (uint32_t)resp[2] << 8 | resp[3];
        fprintf(stderr, "cobcics bridge: unhandled CICS condition %u\n", code);
        exit(2);
    }
}

void COBCICS_RECEIVE_STATUS(unsigned char *resp, unsigned char *resp2) {
    if (!started)
        failure("RECEIVE_STATUS before START");
    const uint32_t code = (input_count == 0 || input_aid == 0x6d ||
                           input_aid == 0x6c || input_aid == 0x6e ||
                           input_aid == 0x6b) ? 36 : 0;
    set_cobol_response(resp, code);
    set_cobol_response(resp2, 0);
}

void COBCICS_SEND_FIELD(const char *name, const unsigned char *source,
                        const unsigned char *length) {
    uint16_t count = cobol_length(length);
    char host_name[9], host_data[MAX_WIRE_DATA + 1];
    if (!started || count > MAX_DATA || output_count >= MAX_FIELDS)
        failure("invalid SEND_FIELD call");
    unsigned nonzero = 0;
    for (unsigned index = 0; index < count; ++index)
        nonzero |= source[index];
    if (!nonzero)
        return; /* BMS low-values leave the physical map's default data. */
    (void)name;
    if (!cob_get_param_str(1, host_name, sizeof host_name) ||
        !cob_get_param_str(2, host_data, sizeof host_data))
        failure("cannot convert output field");
    if (screen_action == 4)
        screen_action = 0;
    for (unsigned index = 0; index < output_count; ++index)
        if (memcmp(output_fields[index].name, host_name, 8) == 0)
            failure("duplicate output field");
    struct field *field = &output_fields[output_count++];
    memcpy(field->name, host_name, 8);
    field->length = (uint16_t)strlen(host_data);
    if (field->length > MAX_WIRE_DATA)
        failure("converted output field exceeds worker limit");
    memcpy(field->data, host_data, field->length);
}

void COBCICS_SEND_ATTRIBUTE(const char *name, const unsigned char *value) {
    char host_name[9];
    if (!started)
        failure("SEND_ATTRIBUTE before START");
    if (*value == 0 || *value == 0xff || *value == 0x80 ||
        *value == ' ' || *value == 0x40)
        return; /* Low/high values or input erased flags do not set attributes. */
    (void)name;
    if (!cob_get_param_str(1, host_name, sizeof host_name))
        failure("cannot convert output attribute name");
    if (output_attribute_count >= MAX_FIELDS)
        failure("too many output attributes");
    for (unsigned index = 0; index < output_attribute_count; ++index)
        if (memcmp(output_attributes[index].name, host_name, 8) == 0)
            failure("duplicate output attribute");
    memcpy(output_attributes[output_attribute_count].name, host_name, 8);
    output_attributes[output_attribute_count++].value = *value;
}

void COBCICS_SEND_EXT_ATTRIBUTE(const char *name, const unsigned char *kind,
                                const unsigned char *value) {
    char host_name[9];
    if (!started)
        failure("SEND_EXT_ATTRIBUTE before START");
    if (*value == 0 || *value == 0xff || *value == 0x80 || *value == ' ' || *value == 0x40)
        return;
    if ((*kind != 'C' || *value < 0xf0) &&
        (*kind != 'H' || !(*value == 0xf0 || *value == 0xf1 ||
                            *value == 0xf2 || *value == 0xf4)))
        failure("unsupported BMS extended attribute");
    (void)name;
    if (!cob_get_param_str(1, host_name, sizeof host_name))
        failure("cannot convert extended attribute name");
    if (output_extended_attribute_count >= MAX_FIELDS * 2)
        failure("too many output extended attributes");
    for (unsigned index = 0; index < output_extended_attribute_count; ++index)
        if (memcmp(output_extended_attributes[index].name, host_name, 8) == 0 &&
            output_extended_attributes[index].kind == *kind)
            failure("duplicate output extended attribute");
    memcpy(output_extended_attributes[output_extended_attribute_count].name, host_name, 8);
    output_extended_attributes[output_extended_attribute_count].kind = *kind;
    output_extended_attributes[output_extended_attribute_count++].value = *value;
}

void COBCICS_SEND_MAP(const unsigned char *action) {
    if (!started || (*action != 0 && *action != 2 && *action != 3))
        failure("invalid SEND_MAP action");
    output_count = 0;
    output_attribute_count = 0;
    output_extended_attribute_count = 0;
    output_text_length = 0;
    screen_action = *action;
}

void COBCICS_SEND_CONTROL(void) {
    if (!started)
        failure("SEND_CONTROL before START");
    output_count = 0;
    output_attribute_count = 0;
    output_extended_attribute_count = 0;
    output_text_length = 0;
    screen_action = 1;
}

void COBCICS_SEND_TEXT(const unsigned char *source, const unsigned char *length,
                       const unsigned char *capacity) {
    uint16_t count = cobol_length(length);
    /* CICS FROM points into contiguous COBOL storage.  GenApp passes the
       first elementary item of a group and includes following items in
       LENGTH, so LENGTH can exceed that item's declared size. */
    (void)capacity;
    if (!started || count > MAX_TEXT)
        failure("invalid SEND TEXT length");
    output_count = 0;
    output_attribute_count = 0;
    output_extended_attribute_count = 0;
    cob_field area = *cob_get_param_field(1, "COBCICS_SEND_TEXT");
    area.data = (unsigned char *)source;
    area.size = count;
    if (!cob_get_field_str(&area, (char *)output_text, sizeof output_text))
        failure("cannot convert SEND TEXT data");
    output_text_length = (uint16_t)strlen((char *)output_text);
    screen_action = 5; /* Erase/Write text and restore keyboard. */
}

void COBCICS_SET_RETURN(const char *transid, const unsigned char *commarea,
                        const unsigned char *length) {
    uint16_t count = cobol_length(length);
    char host_transid[5];
    int seen_space = 0;
    if (!started || link_depth != screen_root_depth || count > MAX_COMMAREA)
        failure("invalid SET_RETURN call");
    (void)transid;
    if (!cob_get_param_str(1, host_transid, sizeof host_transid))
        failure("cannot convert RETURN TRANSID from COBOL data");
    for (unsigned index = 0; index < 4; ++index) {
        if (host_transid[index] == ' ') {
            if (index == 0)
                failure("invalid TRANSID");
            seen_space = 1;
        } else if (seen_space ||
                   !((host_transid[index] >= 'A' && host_transid[index] <= 'Z') ||
                     (host_transid[index] >= '0' && host_transid[index] <= '9')))
            failure("invalid TRANSID");
    }
    memcpy(output_transid, host_transid, 4);
    output_commarea_length = count;
    memcpy(output_commarea, commarea, count);
}

void COBCICS_SET_IMMEDIATE(void) {
    if (!started || link_depth != screen_root_depth || output_transid[0] == ' ')
        failure("IMMEDIATE requires a top-level RETURN TRANSID");
    return_immediate = 1;
}

void COBCICS_FINISH(void) {
    if (!started)
        failure("FINISH before START");
#ifdef COBCICS_DB2SQL
    if (DB2SQL_END_TASK(1) != 0)
        failure("Db2 task commit failed");
#endif
#ifdef COBCICS_GENAPP
    if (COBCICS_START_FLUSH() != 0)
        failure("CICS START scheduling failed");
#endif
    write_bytes("CCO7", 4);
    write_u16((uint16_t)output_count);
    for (unsigned index = 0; index < output_count; ++index) {
        const struct field *field = &output_fields[index];
        write_bytes(field->name, 8);
        write_u16(field->length);
        write_bytes(field->data, field->length);
    }
    write_u16((uint16_t)output_attribute_count);
    for (unsigned index = 0; index < output_attribute_count; ++index) {
        write_bytes(output_attributes[index].name, 8);
        write_bytes(&output_attributes[index].value, 1);
    }
    write_u16((uint16_t)output_extended_attribute_count);
    for (unsigned index = 0; index < output_extended_attribute_count; ++index) {
        write_bytes(output_extended_attributes[index].name, 8);
        write_bytes(&output_extended_attributes[index].kind, 1);
        write_bytes(&output_extended_attributes[index].value, 1);
    }
    write_bytes(&screen_action, 1);
    write_bytes(&return_immediate, 1);
    write_bytes(output_transid, 4);
    write_u16(output_commarea_length);
    write_bytes(output_commarea, output_commarea_length);
    write_u16(output_text_length);
    write_bytes(output_text, output_text_length);
    if (fflush(protocol_stdout) != 0)
        failure("failed to flush worker response");
}

void COBCICS_RETURN(void) {
    if (!started)
        failure("RETURN before START");
    if (link_depth == 0 && active_child < 0)
        COBCICS_FINISH();
}
