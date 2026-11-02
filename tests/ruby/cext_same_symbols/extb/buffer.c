/* the same global names as exta/buffer.c, with different bodies */
const char *buf_owner = "extb";
int bufgrow(int n) { return n + 100; }
