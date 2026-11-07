/* Match OTP 29.1.1 openssl_config.h:114-131, 312-335, 369-377 and
 * algorithms.c:718-743. The preprocessor reads the linked OpenSSL headers. */
#include <openssl/opensslv.h>
#include <openssl/rsa.h>

#if (!defined(LIBRESSL_VERSION_NUMBER) && OPENSSL_VERSION_NUMBER >= 0x10000000L) || (defined(LIBRESSL_VERSION_NUMBER) && LIBRESSL_VERSION_NUMBER >= 0x3050000fL)
#if OPENSSL_VERSION_NUMBER >= 0x10000000L && defined(RSA_PKCS1_PSS_PADDING)
PBEAM_RSA_OPTION("rsa_pkcs1_pss_padding")
PBEAM_RSA_OPTION("rsa_pss_saltlen")
#endif
#if OPENSSL_VERSION_NUMBER >= 0x10001000L
PBEAM_RSA_OPTION("rsa_mgf1_md")
#ifdef RSA_PKCS1_OAEP_PADDING
PBEAM_RSA_OPTION("rsa_pkcs1_oaep_padding")
#endif
#endif
#if OPENSSL_VERSION_NUMBER >= 0x10100000L && !defined(LIBRESSL_VERSION_NUMBER)
PBEAM_RSA_OPTION("rsa_oaep_label")
PBEAM_RSA_OPTION("rsa_oaep_md")
#endif
PBEAM_RSA_OPTION("signature_md")
#endif
PBEAM_RSA_OPTION("rsa_pkcs1_padding")
PBEAM_RSA_OPTION("rsa_x931_padding")
#if !defined(LIBRESSL_VERSION_NUMBER) && defined(RSA_SSLV23_PADDING)
PBEAM_RSA_OPTION("rsa_sslv23_padding")
#endif
PBEAM_RSA_OPTION("rsa_no_padding")
