/* COBOL source branch counters.  The compiler emits stable build/outcome IDs;
   the map containing source positions is written by cobc. */
#include "config.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/stat.h>
#include <errno.h>
#ifdef _WIN32
#include <direct.h>
#include <process.h>
#else
#include <unistd.h>
#endif

#define COB_LIB_EXPIMP
#include "coblocal.h"

struct cob_cov_counter {
	struct cob_cov_counter *next;
	char build_id[17];
	unsigned int outcome_id;
	unsigned long long count;
};

static struct cob_cov_counter *cov_counters;
static int cov_registered;
static int cov_dirty;
static unsigned int cov_file_number;

void
cob_cov_hit (const char *build_id, unsigned int outcome_id)
{
	struct cob_cov_counter *entry;
	if (!build_id || strlen (build_id) != 16) return;
	for (entry = cov_counters; entry; entry = entry->next) {
		if (entry->outcome_id == outcome_id
		 && strcmp (entry->build_id, build_id) == 0)
			break;
	}
	if (!entry) {
		entry = calloc (1, sizeof (*entry));
		if (!entry) return;
		memcpy (entry->build_id, build_id, 17);
		entry->outcome_id = outcome_id;
		entry->next = cov_counters;
		cov_counters = entry;
	}
	if (entry->count != ~0ULL) ++entry->count;
	cov_dirty = 1;
	if (!cov_registered) {
		(void)atexit (cob_cov_flush);
		cov_registered = 1;
	}
}

void
cob_cov_flush (void)
{
	const char *directory = getenv ("COB_COVERAGE_DIR");
	struct cob_cov_counter *entry;
	char *path;
	char *temporary;
	size_t length;
	FILE *file;
	long pid;
	int failed;
	if (!cov_dirty) return;
	if (!directory || !*directory) directory = "cobol-coverage";
#ifdef _WIN32
	if (_mkdir (directory) != 0 && errno != EEXIST) {
		fprintf (stderr, "cobcov: cannot create %s\n", directory);
		return;
	}
	pid = (long)_getpid ();
#else
	if (mkdir (directory, 0777) != 0 && errno != EEXIST) {
		fprintf (stderr, "cobcov: cannot create %s\n", directory);
		return;
	}
	pid = (long)getpid ();
#endif
	length = strlen (directory) + 80;
	path = malloc (length);
	if (!path) return;
	temporary = malloc (length);
	if (!temporary) {
		free (path);
		return;
	}
	for (;;) {
		FILE *existing;
		unsigned int number = cov_file_number++;
		snprintf (path, length, "%s/cobcov-%ld-%ld-%u.counts",
		          directory, (long)time (NULL), pid, number);
		existing = fopen (path, "r");
		if (!existing) break;
		fclose (existing);
	}
	snprintf (temporary, length, "%s.tmp", path);
	file = fopen (temporary, "w");
	if (!file) {
		fprintf (stderr, "cobcov: cannot write %s\n", temporary);
		free (temporary);
		free (path);
		return;
	}
	fputs ("COBCOV1\n", file);
	for (entry = cov_counters; entry; entry = entry->next) {
		if (entry->count) {
			fprintf (file, "%s\t%u\t%llu\n", entry->build_id,
			         entry->outcome_id, entry->count);
		}
	}
	failed = ferror (file);
	if (fclose (file) != 0) failed = 1;
	if (failed) {
		fprintf (stderr, "cobcov: cannot finish %s\n", path);
		remove (temporary);
	} else if (rename (temporary, path) != 0) {
		fprintf (stderr, "cobcov: cannot publish %s\n", path);
		remove (temporary);
	} else {
		for (entry = cov_counters; entry; entry = entry->next)
			entry->count = 0;
		cov_dirty = 0;
	}
	free (temporary);
	free (path);
}
