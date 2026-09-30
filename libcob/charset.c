/*
   Copyright (C) 2026 Free Software Foundation, Inc.

   This file is part of GnuCOBOL.

   The GnuCOBOL runtime is free software: you can redistribute it
   and/or modify it under the terms of the GNU Lesser General Public License
   as published by the Free Software Foundation, either version 3 of the
   License, or (at your option) any later version.

   GnuCOBOL is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU Lesser General Public License for more details.

   You should have received a copy of the GNU Lesser General Public License
   along with GnuCOBOL.  If not, see <https://www.gnu.org/licenses/>.
*/

/* Character set descriptors for COBOL data ("data codeset"),
   see docs/proposal/ebcdic-internal-encoding-ja.md */

#include "config.h"

#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#ifdef	HAVE_ICONV
#include <iconv.h>
#endif
#ifdef	HAVE_LANGINFO_CODESET
#include <langinfo.h>
#endif

/* include internal and external libcob definitions, forcing exports */
#define	COB_LIB_EXPIMP
#include "coblocal.h"

/* tables for the native (= host) character set, filled by cob_init_charset
   from C character constants, so they are correct both on ASCII and
   on EBCDIC hosts */
static unsigned char	native_ctype[256];
static unsigned char	native_upper[256];
static unsigned char	native_lower[256];
static char		native_spaces[COB_SPACES_ALPHABETIC_BYTE_LENGTH];
static char		native_zeroes[COB_ZEROES_ALPHABETIC_BYTE_LENGTH];

const cob_charset	cob_charset_native = {
	"NATIVE",
	0,
#ifdef	COB_EBCDIC_HOST
	COB_CS_EBCDIC,
#else
	0,
#endif
	' ', '0', '+', '-', '.', ',', '*', '/', '"', '\'',
	'C', 'R', 'D', 'B',
	0, 0,
	{ 0, 0 },
	native_spaces,
	native_zeroes,
	native_ctype,
	native_upper,
	native_lower,
	NULL,
	NULL,
	NULL
};

static void
init_native_tables (void)
{
	static const unsigned char	digits[] = "0123456789";
	static const unsigned char	upper[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
	static const unsigned char	lower[] = "abcdefghijklmnopqrstuvwxyz";
	size_t	i;

	memset (native_spaces, ' ', sizeof (native_spaces));
	memset (native_zeroes, '0', sizeof (native_zeroes));
	memset (native_ctype, 0, sizeof (native_ctype));
	for (i = 0; i < 256; i++) {
		native_upper[i] = (unsigned char)i;
		native_lower[i] = (unsigned char)i;
	}
	for (i = 0; i < 10; i++) {
		native_ctype[digits[i]] |= COB_CT_DIGIT;
	}
	for (i = 0; i < 26; i++) {
		native_ctype[upper[i]] |= COB_CT_UPPER;
		native_ctype[lower[i]] |= COB_CT_LOWER;
		native_upper[lower[i]] = upper[i];
		native_lower[upper[i]] = lower[i];
	}
	native_ctype[(unsigned char)' '] |= COB_CT_SPACE | COB_CT_WHITE;
	native_ctype[(unsigned char)'\t'] |= COB_CT_WHITE;
	native_ctype[(unsigned char)'\n'] |= COB_CT_WHITE;
	native_ctype[(unsigned char)'\v'] |= COB_CT_WHITE;
	native_ctype[(unsigned char)'\f'] |= COB_CT_WHITE;
	native_ctype[(unsigned char)'\r'] |= COB_CT_WHITE;
}

/* EBCDIC data charsets */

#include "charset_tables.h"

/* SPACE / ZERO buffers, identical for all EBCDIC charsets */
static char		ebcdic_spaces[COB_SPACES_ALPHABETIC_BYTE_LENGTH];
static char		ebcdic_zeroes[COB_ZEROES_ALPHABETIC_BYTE_LENGTH];

/* the characters used by the runtime are invariant in all EBCDIC charsets */
#define COB_EBCDIC_CHARSET(cname, name, ccsid, flags, so, si, dbcs_space, iconv_name) \
const cob_charset	cob_charset_##cname = { \
	name, ccsid, COB_CS_EBCDIC | (flags), \
	0x40, 0xF0, 0x4E, 0x60, 0x4B, 0x6B, 0x5C, 0x61, 0x7F, 0x7D, \
	0xC3, 0xD9, 0xC4, 0xC2, \
	so, si, \
	{ dbcs_space, dbcs_space }, \
	ebcdic_spaces, \
	ebcdic_zeroes, \
	cname##_ctype, \
	cname##_upper, \
	cname##_lower, \
	cname##_to_host, \
	cname##_from_host, \
	iconv_name \
}

COB_EBCDIC_CHARSET (ibm037, "IBM-037", 37, 0, 0, 0, 0, "IBM037");
COB_EBCDIC_CHARSET (ibm930, "IBM-930", 930, COB_CS_MIXED, 0x0E, 0x0F, 0x40, "IBM930");
COB_EBCDIC_CHARSET (ibm939, "IBM-939", 939, COB_CS_MIXED, 0x0E, 0x0F, 0x40, "IBM939");
COB_EBCDIC_CHARSET (ibm1390, "IBM-1390", 1390, COB_CS_MIXED, 0x0E, 0x0F, 0x40, "IBM1390");
COB_EBCDIC_CHARSET (ibm1399, "IBM-1399", 1399, COB_CS_MIXED, 0x0E, 0x0F, 0x40, "IBM1399");

static const cob_charset	*const known_charsets[] = {
	&cob_charset_ibm037,
	&cob_charset_ibm930,
	&cob_charset_ibm939,
	&cob_charset_ibm1390,
	&cob_charset_ibm1399,
	NULL
};

/* get a data charset by its name, for example "NATIVE", "IBM-1399",
   "IBM1399", "CP1399", "CCSID1399" or "1399" (case-insensitive);
   returns NULL if unknown */
const cob_charset *
cob_get_charset (const char *name)
{
	const char	*p = name;
	unsigned int	ccsid = 0;
	size_t		i;

	if (!name || !*name) {
		return NULL;
	}
	if (!strcasecmp (name, "NATIVE")) {
		return &cob_charset_native;
	}
	/* skip known prefixes */
	if (!strncmp (p, "IBM", 3) || !strncmp (p, "ibm", 3)) {
		p += 3;
	} else if (!strncmp (p, "CCSID", 5) || !strncmp (p, "ccsid", 5)) {
		p += 5;
	} else if (!strncmp (p, "CP", 2) || !strncmp (p, "cp", 2)) {
		p += 2;
	}
	if (*p == '-' || *p == '_') {
		p++;
	}
	if (!*p) {
		return NULL;
	}
	for (; *p; p++) {
		if (*p < '0' || *p > '9') {
			return NULL;
		}
		ccsid = ccsid * 10 + COB_D2I (*p);
		if (ccsid > 99999) {
			return NULL;
		}
	}
	for (i = 0; known_charsets[i]; i++) {
		if (known_charsets[i]->ccsid == ccsid) {
			return known_charsets[i];
		}
	}
	return NULL;
}

/* return the data charset of the given module, native if unset */
const cob_charset *
cob_module_charset (const cob_module *module)
{
	if (module && module->charset) {
		return module->charset;
	}
	return &cob_charset_native;
}

void
cob_init_charset (void)
{
	init_native_tables ();
	memset (ebcdic_spaces, 0x40, sizeof (ebcdic_spaces));
	memset (ebcdic_zeroes, 0xF0, sizeof (ebcdic_zeroes));
}

/* convert 'size' bytes of 'data' in place from host to data charset */
void
cob_charset_from_host (const cob_charset *cs, unsigned char *data, size_t size)
{
	if (cs->from_host) {
		unsigned char	*end = data + size;
		for (; data < end; data++) {
			*data = cs->from_host[*data];
		}
	}
}

/* convert 'size' bytes of 'data' in place from data to host charset */
void
cob_charset_to_host (const cob_charset *cs, unsigned char *data, size_t size)
{
	if (cs->to_host) {
		unsigned char	*end = data + size;
		for (; data < end; data++) {
			*data = cs->to_host[*data];
		}
	}
}

/* return a field with the content of 'f' in host charset:
   either 'f' itself (no conversion necessary) or 'tmp' with allocated
   and converted data, to be released by cob_charset_field_release */
cob_field *
cob_charset_field_to_host (const cob_charset *cs, const cob_field *f, cob_field *tmp)
{
	if (!f || !cs->to_host || !f->data || f->size == 0) {
		return (cob_field *)f;
	}
	*tmp = *f;
	tmp->data = cob_fast_malloc (f->size);
	memcpy (tmp->data, f->data, f->size);
	cob_charset_to_host (cs, tmp->data, f->size);
	return tmp;
}

void
cob_charset_field_release (const cob_field *orig, cob_field *used)
{
	if (used && used != orig) {
		cob_free (used->data);
	}
}

/* Text conversion between data and host charset, used at the boundaries
   to the "outside world" where the text may contain DBCS characters
   (DISPLAY, ACCEPT, LINE SEQUENTIAL files, environment, command line);
   uses iconv when available (the host charset is taken from the
   environment variable COB_HOST_CODESET or from the locale), otherwise
   the single byte tables of the charset */

static int	host_codeset_checked = 0;
static char	*host_codeset = NULL;
static int	host_codeset_explicit = 0;

#ifdef	HAVE_ICONV
#define MAX_ICONV_CACHE 8
static struct iconv_cache_entry {
	const cob_charset	*cs;
	iconv_t		to_host;
	iconv_t		from_host;
	int		failed;
} iconv_cache[MAX_ICONV_CACHE];
static int	iconv_cache_count = 0;
#endif

static void
check_host_codeset (void)
{
	const char	*name;
	if (host_codeset_checked) {
		return;
	}
	host_codeset_checked = 1;
	name = getenv ("COB_HOST_CODESET");
	if (name && *name) {
		host_codeset_explicit = 1;
	} else {
#ifdef	HAVE_LANGINFO_CODESET
		name = nl_langinfo (CODESET);
#else
		name = NULL;
#endif
	}
	if (name && *name) {
		host_codeset = cob_strdup (name);
	}
}

/* return the iconv name of the host charset, NULL if the single byte
   tables are to be used */
const char *
cob_get_host_codeset (void)
{
	check_host_codeset ();
	if (!host_codeset) {
		return NULL;
	}
	/* an ASCII locale (like "C") cannot represent more than the tables,
	   so only use iconv if explicit requested */
	if (!host_codeset_explicit
	 && (!strcmp (host_codeset, "ANSI_X3.4-1968")
	  || !strcmp (host_codeset, "US-ASCII")
	  || !strcmp (host_codeset, "ASCII")
	  || !strcmp (host_codeset, "646"))) {
		return NULL;
	}
	return host_codeset;
}

#ifdef	HAVE_ICONV
static struct iconv_cache_entry *
get_iconv (const cob_charset *cs)
{
	const char	*host = cob_get_host_codeset ();
	int		i;

	if (!cs->iconv_name || !host) {
		return NULL;
	}
	for (i = 0; i < iconv_cache_count; i++) {
		if (iconv_cache[i].cs == cs) {
			return iconv_cache[i].failed ? NULL : &iconv_cache[i];
		}
	}
	if (iconv_cache_count == MAX_ICONV_CACHE) {
		return NULL;
	}
	i = iconv_cache_count++;
	iconv_cache[i].cs = cs;
	iconv_cache[i].to_host = iconv_open (host, cs->iconv_name);
	iconv_cache[i].from_host = iconv_open (cs->iconv_name, host);
	if (iconv_cache[i].to_host == (iconv_t)-1
	 || iconv_cache[i].from_host == (iconv_t)-1) {
		iconv_cache[i].failed = 1;
		return NULL;
	}
	return &iconv_cache[i];
}

/* convert with iconv, replacing invalid / unconvertible input by 'subst';
   returns the number of bytes stored in 'out' */
static size_t
convert_iconv (iconv_t cd, const unsigned char *src, const size_t len,
	unsigned char *out, const size_t outsize, const unsigned char subst)
{
	ICONV_CONST char	*in = (ICONV_CONST char *)src;
	char	*o = (char *)out;
	size_t	inleft = len;
	size_t	oleft = outsize;

	(void)iconv (cd, NULL, NULL, NULL, NULL);
	while (inleft > 0) {
		if (iconv (cd, &in, &inleft, &o, &oleft) != (size_t)-1) {
			break;
		}
		if (errno == E2BIG || oleft == 0) {
			break;
		}
		/* invalid or incomplete input: substitute and go on */
		in++;
		inleft--;
		*o++ = (char)subst;
		oleft--;
	}
	/* write final shift sequence, if any */
	(void)iconv (cd, NULL, NULL, &o, &oleft);
	return (size_t)(o - (char *)out);
}
#endif

/* convert text from data to host charset;
   returns an allocated buffer (to be freed with cob_free) with a
   terminating NUL and its size (without that) in 'outsize' */
unsigned char *
cob_charset_text_to_host (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out;
#ifdef	HAVE_ICONV
	struct iconv_cache_entry	*ic = get_iconv (cs);
	if (ic) {
		/* EBCDIC DBCS (2 bytes) or SBCS katakana (1 byte) may need
		   up to 3 bytes in UTF-8 */
		const size_t	max = size * 4 + 16;
		out = cob_fast_malloc (max + 1);
		*outsize = convert_iconv (ic->to_host, src, size, out, max,
			(unsigned char)'?');
		out[*outsize] = 0;
		return out;
	}
#endif
	out = cob_fast_malloc (size + 1);
	memcpy (out, src, size);
	cob_charset_to_host (cs, out, size);
	out[size] = 0;
	*outsize = size;
	return out;
}

/* convert text from host to data charset;
   returns an allocated buffer (to be freed with cob_free) and its size
   in 'outsize' - which may differ from 'size' (SO/SI for DBCS) */
unsigned char *
cob_charset_text_from_host (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out;
#ifdef	HAVE_ICONV
	struct iconv_cache_entry	*ic = get_iconv (cs);
	if (ic) {
		/* a DBCS character needs up to 4 bytes (SO + 2 + SI) */
		const size_t	max = size * 2 + 16;
		out = cob_fast_malloc (max + 1);
		*outsize = convert_iconv (ic->from_host, src, size, out, max,
			COB_CS_FROM_HOST (cs, '?'));
		out[*outsize] = 0;
		return out;
	}
#endif
	out = cob_fast_malloc (size + 1);
	memcpy (out, src, size);
	cob_charset_from_host (cs, out, size);
	out[size] = 0;
	*outsize = size;
	return out;
}

/* convert mixed SBCS / DBCS data (with SO/SI) to pure DBCS data:
   SO and SI are removed, single byte spaces are changed to DBCS spaces,
   other single byte characters are dropped;
   returns an allocated buffer (to be freed with cob_free) and its size */
unsigned char *
cob_charset_mixed_to_dbcs (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out = cob_fast_malloc (size * 2 + 1);
	size_t		i, o = 0;
	int		in_dbcs = 0;

	for (i = 0; i < size; i++) {
		if (src[i] == cs->so) {
			in_dbcs = 1;
		} else if (src[i] == cs->si) {
			in_dbcs = 0;
		} else if (in_dbcs) {
			out[o++] = src[i];
		} else if (src[i] == cs->space) {
			out[o++] = cs->dbcs_space[0];
			out[o++] = cs->dbcs_space[1];
		}
	}
	out[o] = 0;
	*outsize = o;
	return out;
}

/* check if text conversion changes the size (uses iconv) */
int
cob_charset_text_is_variable (const cob_charset *cs)
{
#ifdef	HAVE_ICONV
	return get_iconv (cs) != NULL;
#else
	COB_UNUSED (cs);
	return 0;
#endif
}

void
cob_exit_charset (void)
{
#ifdef	HAVE_ICONV
	int	i;
	for (i = 0; i < iconv_cache_count; i++) {
		if (!iconv_cache[i].failed) {
			iconv_close (iconv_cache[i].to_host);
			iconv_close (iconv_cache[i].from_host);
		} else {
			if (iconv_cache[i].to_host != (iconv_t)-1) {
				iconv_close (iconv_cache[i].to_host);
			}
			if (iconv_cache[i].from_host != (iconv_t)-1) {
				iconv_close (iconv_cache[i].from_host);
			}
		}
	}
	iconv_cache_count = 0;
#endif
	if (host_codeset) {
		cob_free (host_codeset);
		host_codeset = NULL;
	}
	host_codeset_checked = 0;
	host_codeset_explicit = 0;
}
