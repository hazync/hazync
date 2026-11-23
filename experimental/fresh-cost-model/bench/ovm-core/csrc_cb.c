extern void hz_fatal(int code);
void secp256k1_default_illegal_callback_fn(const char *str, void *data) { (void)str; (void)data; hz_fatal(1); }
void secp256k1_default_error_callback_fn(const char *str, void *data) { (void)str; (void)data; hz_fatal(2); }
