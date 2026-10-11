#define COBCICS_IMSDB
#include "bridge.c"

void TESTCICSSTART(void) { started = 1; }
int TESTIMSCOM(void) { return commit_uow(); }
int TESTIMSROL(void) { return rollback_uow(); }
