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
}
