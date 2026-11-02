/* rinku and redcarpet both vendor Sundown's buffer.c: the same global names in two
   extensions (here with different bodies, so a mix-up shows) */
const char *buf_owner = "exta";
int bufgrow(int n) { return n + 1; }
