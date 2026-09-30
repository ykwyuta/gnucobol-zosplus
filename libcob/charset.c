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

/* User-defined DBCS characters ("Gaiji"):
   iconv maps the IBM user-defined area 0x6941 - 0x7FFE to the Unicode
   private use area U+E000 - U+F111; a table given with the environment
   variable COB_DBCS_USER_TABLE maps these to other Unicode characters
   (for UTF-8 hosts), lines have the format
     <EBCDIC DBCS code in hex> <Unicode code point, optionally U+>
   for example "6941 U+9AD9" */

#define USER_PUA_FIRST	0xE000U
#define USER_ROW_SIZE	190U	/* trail bytes 0x41 - 0xFE */
#define USER_COUNT	(23U * USER_ROW_SIZE)	/* lead bytes 0x69 - 0x7F */

static int		user_table_checked = 0;
static unsigned int	*user_to_ucs = NULL;	/* index: PUA code - USER_PUA_FIRST */

static void
load_user_table (void)
{
	const char	*name;
	FILE	*fp;
	char	line[COB_MINI_BUFF];
	int	line_no = 0;

	user_table_checked = 1;
	name = getenv ("COB_DBCS_USER_TABLE");
	if (!name || !*name) {
		return;
	}
	fp = fopen (name, "r");
	if (!fp) {
		cob_runtime_warning (_("cannot open %s '%s'"), "COB_DBCS_USER_TABLE", name);
		return;
	}
	user_to_ucs = cob_malloc (USER_COUNT * sizeof (unsigned int));
	while (fgets (line, (int)sizeof (line), fp)) {
		char		*p = line, *e;
		unsigned long	code, ucs;
		line_no++;
		while (*p == ' ' || *p == '\t') p++;
		if (*p == '#' || *p == '\n' || *p == '\r' || *p == 0) {
			continue;
		}
		code = strtoul (p, &e, 16);
		p = e;
		while (*p == ' ' || *p == '\t') p++;
		if (*p == 'U' || *p == 'u') {
			p++;
			if (*p == '+') p++;
		}
		ucs = strtoul (p, &e, 16);
		if (e == p
		 || (code >> 8) < 0x69 || (code >> 8) > 0x7F
		 || (code & 0xFF) < 0x41 || (code & 0xFF) > 0xFE
		 || ucs == 0 || ucs > 0x10FFFF) {
			cob_runtime_warning (_("invalid entry in %s '%s', line %d"),
				"COB_DBCS_USER_TABLE", name, line_no);
			continue;
		}
		user_to_ucs[((code >> 8) - 0x69) * USER_ROW_SIZE + (code & 0xFF) - 0x41]
			= (unsigned int)ucs;
	}
	fclose (fp);
}

/* check if the user table is to be applied (DBCS charset, UTF-8 host) */
static int
use_user_table (const cob_charset *cs)
{
	const char	*host;
	if (!(cs->flags & COB_CS_MIXED)) {
		return 0;
	}
	if (!user_table_checked) {
		load_user_table ();
	}
	if (!user_to_ucs) {
		return 0;
	}
	host = cob_get_host_codeset ();
	return host && (!strcasecmp (host, "UTF-8") || !strcasecmp (host, "UTF8"));
}

/* decode one UTF-8 character, returns its length (1 for invalid data) */
static size_t
utf8_decode (const unsigned char *p, const unsigned char *end, unsigned int *ucs)
{
	size_t	len, i;
	if (*p < 0x80) {
		*ucs = *p;
		return 1;
	} else if ((*p & 0xE0) == 0xC0) {
		len = 2;
		*ucs = *p & 0x1F;
	} else if ((*p & 0xF0) == 0xE0) {
		len = 3;
		*ucs = *p & 0x0F;
	} else if ((*p & 0xF8) == 0xF0) {
		len = 4;
		*ucs = *p & 0x07;
	} else {
		*ucs = *p;
		return 1;
	}
	if (p + len > end) {
		*ucs = *p;
		return 1;
	}
	for (i = 1; i < len; i++) {
		if ((p[i] & 0xC0) != 0x80) {
			*ucs = *p;
			return 1;
		}
		*ucs = (*ucs << 6) | (p[i] & 0x3F);
	}
	return len;
}

static size_t
utf8_encode (unsigned int ucs, unsigned char *out)
{
	if (ucs < 0x80) {
		out[0] = (unsigned char)ucs;
		return 1;
	} else if (ucs < 0x800) {
		out[0] = (unsigned char)(0xC0 | (ucs >> 6));
		out[1] = (unsigned char)(0x80 | (ucs & 0x3F));
		return 2;
	} else if (ucs < 0x10000) {
		out[0] = (unsigned char)(0xE0 | (ucs >> 12));
		out[1] = (unsigned char)(0x80 | ((ucs >> 6) & 0x3F));
		out[2] = (unsigned char)(0x80 | (ucs & 0x3F));
		return 3;
	}
	out[0] = (unsigned char)(0xF0 | (ucs >> 18));
	out[1] = (unsigned char)(0x80 | ((ucs >> 12) & 0x3F));
	out[2] = (unsigned char)(0x80 | ((ucs >> 6) & 0x3F));
	out[3] = (unsigned char)(0x80 | (ucs & 0x3F));
	return 4;
}

/* replace characters in UTF-8 text using the user table:
   'to_host' set: private use area -> user character, otherwise reverse;
   returns an allocated buffer and its size */
static unsigned char *
apply_user_table (const unsigned char *src, const size_t size,
	const int to_host, size_t *outsize)
{
	/* a 3 byte PUA character may become 4 bytes */
	unsigned char	*out = cob_fast_malloc (size * 2 + 1);
	const unsigned char	*p = src, *end = src + size;
	size_t		o = 0;

	while (p < end) {
		unsigned int	ucs;
		const size_t	len = utf8_decode (p, end, &ucs);
		unsigned int	mapped = 0;
		if (len > 1) {
			if (to_host) {
				if (ucs >= USER_PUA_FIRST && ucs < USER_PUA_FIRST + USER_COUNT) {
					mapped = user_to_ucs[ucs - USER_PUA_FIRST];
				}
			} else {
				unsigned int	i;
				for (i = 0; i < USER_COUNT; i++) {
					if (user_to_ucs[i] == ucs) {
						mapped = USER_PUA_FIRST + i;
						break;
					}
				}
			}
		}
		if (mapped) {
			o += utf8_encode (mapped, out + o);
		} else {
			memcpy (out + o, p, len);
			o += len;
		}
		p += len;
	}
	out[o] = 0;
	*outsize = o;
	return out;
}

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
		if (use_user_table (cs)) {
			unsigned char	*mapped = apply_user_table (out, *outsize, 1, outsize);
			cob_free (out);
			out = mapped;
		}
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
		unsigned char	*mapped = NULL;
		size_t		in_size = size;
		if (use_user_table (cs)) {
			mapped = apply_user_table (src, size, 0, &in_size);
			src = mapped;
		}
		out = cob_fast_malloc (max + 1);
		*outsize = convert_iconv (ic->from_host, src, in_size, out, max,
			COB_CS_FROM_HOST (cs, '?'));
		out[*outsize] = 0;
		if (mapped) {
			cob_free (mapped);
		}
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

/* conversion between arbitrary charsets (given by iconv names),
   used for national (UTF-16) data */

#ifdef	HAVE_ICONV
#define MAX_ICONV_PAIRS 16
static struct iconv_pair_entry {
	char		*to;
	char		*from;
	iconv_t		cd;
} iconv_pairs[MAX_ICONV_PAIRS];
static int	iconv_pair_count = 0;

static iconv_t
get_iconv_pair (const char *to, const char *from)
{
	iconv_t		cd;
	int		i;
	for (i = 0; i < iconv_pair_count; i++) {
		if (!strcmp (iconv_pairs[i].to, to)
		 && !strcmp (iconv_pairs[i].from, from)) {
			return iconv_pairs[i].cd;
		}
	}
	cd = iconv_open (to, from);
	if (iconv_pair_count < MAX_ICONV_PAIRS) {
		i = iconv_pair_count++;
		iconv_pairs[i].to = cob_strdup (to);
		iconv_pairs[i].from = cob_strdup (from);
		iconv_pairs[i].cd = cd;
	} else if (cd != (iconv_t)-1) {
		/* cache full: keep it open for the last slot */
		i = MAX_ICONV_PAIRS - 1;
		if (iconv_pairs[i].cd != (iconv_t)-1) {
			iconv_close (iconv_pairs[i].cd);
		}
		cob_free (iconv_pairs[i].to);
		cob_free (iconv_pairs[i].from);
		iconv_pairs[i].to = cob_strdup (to);
		iconv_pairs[i].from = cob_strdup (from);
		iconv_pairs[i].cd = cd;
	}
	return cd;
}

/* convert with iconv, replacing invalid / unconvertible input by 'subst'
   (with a size of 'subst_len'), for UTF-16 input the invalid unit
   is skipped as a whole */
static size_t
convert_iconv_subst (iconv_t cd, const unsigned char *src, const size_t len,
	unsigned char *out, const size_t outsize,
	const unsigned char *subst, const size_t subst_len, const size_t unit)
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
		if (errno == E2BIG || oleft < subst_len) {
			break;
		}
		/* invalid or incomplete input: substitute and go on */
		if (inleft < unit) {
			inleft = 0;
		} else {
			in += unit;
			inleft -= unit;
		}
		memcpy (o, subst, subst_len);
		o += subst_len;
		oleft -= subst_len;
	}
	/* write final shift sequence (SI), if any */
	(void)iconv (cd, NULL, NULL, &o, &oleft);
	return (size_t)(o - (char *)out);
}
#endif

/* iconv name of a CCSID, for example 1399 -> "IBM1399";
   'buff' needs at least 16 bytes */
const char *
cob_charset_ccsid_name (const int ccsid, char *buff)
{
	switch (ccsid) {
	case 1200:
	case 1201:
	case 13488:
		return "UTF-16BE";
	case 1202:
		return "UTF-16LE";
	case 1208:
		return "UTF-8";
	case 367:
		return "ASCII";
	case 819:
		return "ISO-8859-1";
	case 932:
		return "CP932";
	case 5050:
	case 954:
		return "EUC-JP";
	default:
		sprintf (buff, "IBM%03d", ccsid);
		return buff;
	}
}

/* convert 'size' bytes from charset 'from' to charset 'to' (iconv names);
   returns an allocated buffer (to be freed with cob_free) and its size,
   NULL if the conversion is not possible */
unsigned char *
cob_charset_convert (const char *to, const char *from,
	const unsigned char *src, const size_t size, size_t *outsize)
{
#ifdef	HAVE_ICONV
	static const unsigned char	subst_utf16[2] = { 0xFF, 0xFD };
	const unsigned char	*subst;
	unsigned char		subst_buff[4];
	size_t			subst_len;
	size_t			unit = 1;
	unsigned char		*out;
	size_t			max;
	iconv_t		cd = get_iconv_pair (to, from);

	if (cd == (iconv_t)-1) {
		return NULL;
	}
	if (!strncmp (from, "UTF-16", 6)) {
		unit = 2;
	}
	if (!strcmp (to, "UTF-16BE")) {
		subst = subst_utf16;
		subst_len = 2;
	} else if (!strcmp (to, "UTF-16LE")) {
		subst_buff[0] = 0xFD;
		subst_buff[1] = 0xFF;
		subst = subst_buff;
		subst_len = 2;
	} else if (!strncmp (to, "IBM", 3)) {
		subst_buff[0] = 0x6F;	/* '?' in all EBCDIC charsets */
		subst = subst_buff;
		subst_len = 1;
	} else {
		subst_buff[0] = '?';
		subst = subst_buff;
		subst_len = 1;
	}
	/* worst cases: 1 byte to 4 bytes (UTF-8 / UTF-16 surrogates),
	   2 bytes to 4 (SO + DBCS + SI) */
	max = size * 4 + 16;
	out = cob_fast_malloc (max + 1);
	*outsize = convert_iconv_subst (cd, src, size, out, max,
		subst, subst_len, unit);
	out[*outsize] = 0;
	return out;
#else
	COB_UNUSED (to);
	COB_UNUSED (from);
	COB_UNUSED (src);
	COB_UNUSED (size);
	COB_UNUSED (outsize);
	return NULL;
#endif
}

/* convert data (in the data charset, with SO/SI for DBCS) to UTF-16 */
unsigned char *
cob_charset_to_utf16 (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out;
	size_t		i;
	if (cs->iconv_name) {
		out = cob_charset_convert ("UTF-16BE", cs->iconv_name, src, size, outsize);
		if (out) {
			return out;
		}
	}
	/* fallback: single byte table to the host (ISO-8859-1 like) */
	out = cob_fast_malloc (size * 2 + 1);
	for (i = 0; i < size; i++) {
		out[i * 2] = 0;
		out[i * 2 + 1] = COB_CS_TO_HOST (cs, src[i]);
	}
	*outsize = size * 2;
	out[*outsize] = 0;
	return out;
}

/* convert UTF-16 data to the data charset (with SO/SI for DBCS) */
unsigned char *
cob_charset_from_utf16 (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out;
	size_t		i;
	if (cs->iconv_name) {
		out = cob_charset_convert (cs->iconv_name, "UTF-16BE", src, size, outsize);
		if (out) {
			return out;
		}
	}
	out = cob_fast_malloc (size / 2 + 1);
	for (i = 0; i + 1 < size; i += 2) {
		out[i / 2] = src[i] ? COB_CS_FROM_HOST (cs, '?')
			: COB_CS_FROM_HOST (cs, src[i + 1]);
	}
	*outsize = size / 2;
	out[*outsize] = 0;
	return out;
}

/* convert UTF-16 data to host text (terminated by NUL) */
unsigned char *
cob_charset_utf16_to_host (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	const char	*host = cob_get_host_codeset ();
	unsigned char	*data, *out;
	size_t		data_size;
	if (host) {
		out = cob_charset_convert (host, "UTF-16BE", src, size, outsize);
		if (out) {
			return out;
		}
	}
	data = cob_charset_from_utf16 (cs, src, size, &data_size);
	out = cob_charset_text_to_host (cs, data, data_size, outsize);
	cob_free (data);
	return out;
}

/* convert host text to UTF-16 data */
unsigned char *
cob_charset_utf16_from_host (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	const char	*host = cob_get_host_codeset ();
	unsigned char	*data, *out;
	size_t		data_size;
	if (host) {
		out = cob_charset_convert ("UTF-16BE", host, src, size, outsize);
		if (out) {
			return out;
		}
	}
	data = cob_charset_text_from_host (cs, src, size, &data_size);
	out = cob_charset_to_utf16 (cs, data, data_size, outsize);
	cob_free (data);
	return out;
}

/* convert pure DBCS data to mixed data by adding SO/SI */
unsigned char *
cob_charset_dbcs_to_mixed (const cob_charset *cs, const unsigned char *src,
	const size_t size, size_t *outsize)
{
	unsigned char	*out = cob_fast_malloc (size + 3);
	out[0] = cs->so;
	memcpy (out + 1, src, size);
	out[size + 1] = cs->si;
	out[size + 2] = 0;
	*outsize = size + 2;
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
	for (i = 0; i < iconv_pair_count; i++) {
		if (iconv_pairs[i].cd != (iconv_t)-1) {
			iconv_close (iconv_pairs[i].cd);
		}
		cob_free (iconv_pairs[i].to);
		cob_free (iconv_pairs[i].from);
	}
	iconv_pair_count = 0;
#endif
	if (host_codeset) {
		cob_free (host_codeset);
		host_codeset = NULL;
	}
	if (user_to_ucs) {
		cob_free (user_to_ucs);
		user_to_ucs = NULL;
	}
	user_table_checked = 0;
	host_codeset_checked = 0;
	host_codeset_explicit = 0;
}
