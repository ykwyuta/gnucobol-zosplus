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
#include <string.h>

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
#define COB_EBCDIC_CHARSET(cname, name, ccsid, flags, so, si, dbcs_space) \
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
	cname##_from_host \
}

COB_EBCDIC_CHARSET (ibm037, "IBM-037", 37, 0, 0, 0, 0);
COB_EBCDIC_CHARSET (ibm930, "IBM-930", 930, COB_CS_MIXED, 0x0E, 0x0F, 0x40);
COB_EBCDIC_CHARSET (ibm939, "IBM-939", 939, COB_CS_MIXED, 0x0E, 0x0F, 0x40);
COB_EBCDIC_CHARSET (ibm1390, "IBM-1390", 1390, COB_CS_MIXED, 0x0E, 0x0F, 0x40);
COB_EBCDIC_CHARSET (ibm1399, "IBM-1399", 1399, COB_CS_MIXED, 0x0E, 0x0F, 0x40);

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
