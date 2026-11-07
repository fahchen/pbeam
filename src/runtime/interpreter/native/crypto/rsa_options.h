/* Match OTP 29.1.1 openssl_config.h:114-131, 312-335, 369-377 and
 * algorithms.c:718-743.
 *
 * The build links the pinned `openssl-src` crate, so the option set is fixed
 * for every build of this binary. The pinned OpenSSL provides every option in
 * this list, and the probe compares the whole list with the pinned ERTS, so no
 * version conditional is needed and none may be added without a consumer. */
PBEAM_RSA_OPTION("rsa_pkcs1_pss_padding")
PBEAM_RSA_OPTION("rsa_pss_saltlen")
PBEAM_RSA_OPTION("rsa_mgf1_md")
PBEAM_RSA_OPTION("rsa_pkcs1_oaep_padding")
PBEAM_RSA_OPTION("rsa_oaep_label")
PBEAM_RSA_OPTION("rsa_oaep_md")
PBEAM_RSA_OPTION("signature_md")
PBEAM_RSA_OPTION("rsa_pkcs1_padding")
PBEAM_RSA_OPTION("rsa_x931_padding")
PBEAM_RSA_OPTION("rsa_no_padding")
