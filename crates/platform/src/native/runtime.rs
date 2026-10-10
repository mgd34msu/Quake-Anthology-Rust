//! Native runtime imports resolved at load and executed only in the owned child.
use super::NativeScalar;

pub const FIRST: u32 = 0x8000_0000;
pub struct Function {
    pub name: &'static [u8],
    pub number: u32,
    pub heap: bool,
    pub provider: &'static [u8],
    pub versions: &'static [&'static [u8]],
    pub parameters: &'static [NativeScalar],
    pub result: NativeScalar,
    pub(crate) operation: Operation,
}
use NativeScalar::{Double, I32, U32, Void, Word};
const LIBC: &[u8] = b"libc.so.6";
const LIBM: &[u8] = b"libm.so.6";
const BASE_VERSION: &[&[u8]] = &[b"GLIBC_2.2.5"];
pub const FUNCTIONS: &[Function] = &[
    Function {
        name: b"memcpy",
        heap: false,
        provider: LIBC,
        versions: &[b"GLIBC_2.2.5", b"GLIBC_2.14"],
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        operation: Operation::Copy,
    },
    Function {
        name: b"memmove",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST,
        parameters: &[Word, Word, Word],
        result: Word,
        operation: Operation::Copy,
    },
    Function {
        name: b"memset",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 1,
        parameters: &[Word, I32, Word],
        result: Word,
        operation: Operation::Fill,
    },
    Function {
        name: b"strncpy",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 2,
        parameters: &[Word, Word, Word],
        result: Word,
        operation: Operation::Strncpy,
    },
    Function {
        name: b"strlen",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 3,
        parameters: &[Word],
        result: Word,
        operation: Operation::Length,
    },
    Function {
        name: b"strcmp",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 4,
        parameters: &[Word, Word],
        result: I32,
        operation: Operation::Compare(Comparison::String),
    },
    Function {
        name: b"memcmp",
        heap: false,
        provider: LIBC,
        versions: BASE_VERSION,
        number: FIRST + 5,
        parameters: &[Word, Word, Word],
        result: I32,
        operation: Operation::Compare(Comparison::Memory),
    },
    Function {
        name: b"sin",
        number: FIRST + 6,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Sin, Double),
    },
    Function {
        name: b"cos",
        number: FIRST + 7,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Cos, Double),
    },
    Function {
        name: b"atan2",
        number: FIRST + 8,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double, Double],
        result: Double,
        operation: Operation::Math(Math::Atan2, Double),
    },
    Function {
        name: b"sqrt",
        number: FIRST + 9,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Sqrt, Double),
    },
    Function {
        name: b"floor",
        number: FIRST + 10,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Floor, Double),
    },
    Function {
        name: b"ceil",
        number: FIRST + 11,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Ceil, Double),
    },
    Function {
        name: b"acos",
        number: FIRST + 12,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Acos, Double),
    },
    Function {
        name: b"fabs",
        number: FIRST + 13,
        heap: false,
        provider: LIBM,
        versions: BASE_VERSION,
        parameters: &[Double],
        result: Double,
        operation: Operation::Math(Math::Absolute, Double),
    },
    Function {
        name: b"malloc",
        number: FIRST + 14,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word],
        result: Word,
        operation: Operation::Malloc,
    },
    Function {
        name: b"calloc",
        number: FIRST + 15,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word, Word],
        result: Word,
        operation: Operation::Calloc,
    },
    Function {
        name: b"realloc",
        number: FIRST + 16,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word, Word],
        result: Word,
        operation: Operation::Realloc,
    },
    Function {
        name: b"free",
        number: FIRST + 17,
        heap: true,
        provider: LIBC,
        versions: BASE_VERSION,
        parameters: &[Word],
        result: Void,
        operation: Operation::Free,
    },
    windows(FIRST + 100, b"msvcp140.dll", b"?_Incref@facet@locale@std@@UEAAXXZ", Operation::Msvc(Msvc::Incref), &[Word], NativeScalar::Void),
    windows(FIRST + 101, b"msvcp140.dll", b"?_Decref@facet@locale@std@@UEAAPEAV_Facet_base@3@XZ", Operation::Msvc(Msvc::Decref), &[Word], NativeScalar::Word),
    windows(FIRST + 102, b"msvcp140.dll", b"??1facet@locale@std@@MEAA@XZ", Operation::Msvc(Msvc::FacetDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 103, b"msvcp140.dll", b"runtime:facet-delete", Operation::Msvc(Msvc::FacetDelete), &[Word, U32], NativeScalar::Word),
    windows(FIRST + 104, b"msvcp140.dll", b"??0facet@locale@std@@IEAA@_K@Z", Operation::Msvc(Msvc::FacetCtor), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 105, b"msvcp140.dll", b"?_Init@locale@std@@CAPEAV_Locimp@12@_N@Z", Operation::Msvc(Msvc::LocaleInit), &[U32], NativeScalar::Word),
    windows(FIRST + 106, b"msvcp140.dll", b"?_Getgloballocale@locale@std@@CAPEAV_Locimp@12@XZ", Operation::Msvc(Msvc::Global), &[], NativeScalar::Word),
    windows(FIRST + 107, b"msvcp140.dll", b"??0_Lockit@std@@QEAA@H@Z", Operation::Msvc(Msvc::Lock), &[Word, I32], NativeScalar::Word),
    windows(FIRST + 108, b"msvcp140.dll", b"??1_Lockit@std@@QEAA@XZ", Operation::Msvc(Msvc::Unlock), &[Word], NativeScalar::Void),
    windows(FIRST + 109, b"msvcp140.dll", b"??0_Locinfo@std@@QEAA@PEBD@Z", Operation::Msvc(Msvc::LocinfoCtor), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 110, b"msvcp140.dll", b"??1_Locinfo@std@@QEAA@XZ", Operation::Msvc(Msvc::LocinfoDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 111, b"msvcp140.dll", b"?_Gettrue@_Locinfo@std@@QEBAPEBDXZ", Operation::Msvc(Msvc::True), &[Word], NativeScalar::Word),
    windows(FIRST + 112, b"msvcp140.dll", b"?_Getfalse@_Locinfo@std@@QEBAPEBDXZ", Operation::Msvc(Msvc::False), &[Word], NativeScalar::Word),
    windows(FIRST + 113, b"msvcp140.dll", b"?_Getlconv@_Locinfo@std@@QEBAPEBUlconv@@XZ", Operation::Msvc(Msvc::Lconv), &[Word], NativeScalar::Word),
    windows(FIRST + 114, b"msvcp140.dll", b"?_Getcvt@_Locinfo@std@@QEBA?AU_Cvtvec@@XZ", Operation::Msvc(Msvc::Cvtvec), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 115, b"msvcp140.dll", b"??1?$basic_ios@DU?$char_traits@D@std@@@std@@UEAA@XZ", Operation::Msvc(Msvc::IosDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 116, b"msvcp140.dll", b"runtime:basic-ios-delete", Operation::Msvc(Msvc::IosDelete), &[Word, U32], NativeScalar::Word),
    windows(FIRST + 117, b"msvcp140.dll", b"??0?$basic_ios@DU?$char_traits@D@std@@@std@@IEAA@XZ", Operation::Msvc(Msvc::IosCtor), &[Word], NativeScalar::Word),
    windows(FIRST + 118, b"msvcp140.dll", b"?rdbuf@?$basic_ios@DU?$char_traits@D@std@@@std@@QEBAPEAV?$basic_streambuf@DU?$char_traits@D@std@@@2@XZ", Operation::Msvc(Msvc::Rdbuf), &[Word], NativeScalar::Word),
    windows(FIRST + 119, b"msvcp140.dll", b"?setstate@?$basic_ios@DU?$char_traits@D@std@@@std@@QEAAXH_N@Z", Operation::Msvc(Msvc::Setstate), &[Word, I32, U32], NativeScalar::Void),
    windows(FIRST + 120, b"msvcp140.dll", b"?good@ios_base@std@@QEBA_NXZ", Operation::Msvc(Msvc::Good), &[Word], NativeScalar::U32),
    windows(FIRST + 121, b"msvcp140.dll", b"??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAA@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z", Operation::Msvc(Msvc::OstreamCtor), &[Word, Word, U32, I32], NativeScalar::Word),
    windows(FIRST + 122, b"msvcp140.dll", b"??0?$basic_iostream@DU?$char_traits@D@std@@@std@@QEAA@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z", Operation::Msvc(Msvc::IostreamCtor), &[Word, Word, I32], NativeScalar::Word),
    windows(FIRST + 123, b"msvcp140.dll", b"??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UEAA@XZ", Operation::Msvc(Msvc::OstreamDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 124, b"msvcp140.dll", b"??1?$basic_iostream@DU?$char_traits@D@std@@@std@@UEAA@XZ", Operation::Msvc(Msvc::IostreamDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 125, b"msvcp140.dll", b"runtime:ostream-delete", Operation::Msvc(Msvc::OstreamDelete), &[Word, U32], NativeScalar::Word),
    windows(FIRST + 126, b"msvcp140.dll", b"??1?$basic_streambuf@DU?$char_traits@D@std@@@std@@UEAA@XZ", Operation::Msvc(Msvc::BufferDtor), &[Word], NativeScalar::Void),
    windows(FIRST + 127, b"msvcp140.dll", b"runtime:streambuf-delete", Operation::Msvc(Msvc::BufferDelete), &[Word, U32], NativeScalar::Word),
    windows(FIRST + 128, b"msvcp140.dll", b"?_Lock@?$basic_streambuf@DU?$char_traits@D@std@@@std@@UEAAXXZ", Operation::Msvc(Msvc::BufferLock), &[Word], NativeScalar::Void),
    windows(FIRST + 129, b"msvcp140.dll", b"?_Unlock@?$basic_streambuf@DU?$char_traits@D@std@@@std@@UEAAXXZ", Operation::Msvc(Msvc::BufferLock), &[Word], NativeScalar::Void),
    windows(FIRST + 130, b"msvcp140.dll", b"runtime:streambuf-overflow", Operation::Msvc(Msvc::BufferOverflow), &[Word, I32], NativeScalar::I32),
    windows(FIRST + 131, b"msvcp140.dll", b"runtime:streambuf-pbackfail", Operation::Msvc(Msvc::BufferOverflow), &[Word, I32], NativeScalar::I32),
    windows(FIRST + 132, b"msvcp140.dll", b"runtime:streambuf-underflow", Operation::Msvc(Msvc::BufferUnderflow), &[Word], NativeScalar::I32),
    windows(FIRST + 133, b"msvcp140.dll", b"?showmanyc@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAA_JXZ", Operation::Msvc(Msvc::Showmany), &[Word], NativeScalar::Word),
    windows(FIRST + 134, b"msvcp140.dll", b"?sync@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAAHXZ", Operation::Msvc(Msvc::Sync), &[Word], NativeScalar::I32),
    windows(FIRST + 135, b"msvcp140.dll", b"?setbuf@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAAPEAV12@PEAD_J@Z", Operation::Msvc(Msvc::Setbuf), &[Word, Word, Word], NativeScalar::Word),
    windows(FIRST + 136, b"msvcp140.dll", b"?imbue@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAAXAEBVlocale@2@@Z", Operation::Msvc(Msvc::Imbue), &[Word, Word], NativeScalar::Void),
    windows(FIRST + 137, b"msvcp140.dll", b"?eback@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Eback), &[Word], NativeScalar::Word),
    windows(FIRST + 138, b"msvcp140.dll", b"?pbase@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Pbase), &[Word], NativeScalar::Word),
    windows(FIRST + 139, b"msvcp140.dll", b"?gptr@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Gptr), &[Word], NativeScalar::Word),
    windows(FIRST + 140, b"msvcp140.dll", b"?pptr@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Pptr), &[Word], NativeScalar::Word),
    windows(FIRST + 141, b"msvcp140.dll", b"?egptr@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Egptr), &[Word], NativeScalar::Word),
    windows(FIRST + 142, b"msvcp140.dll", b"?epptr@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEBAPEADXZ", Operation::Msvc(Msvc::Epptr), &[Word], NativeScalar::Word),
    windows(FIRST + 143, b"msvcp140.dll", b"?uflow@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAAHXZ", Operation::Msvc(Msvc::Uflow), &[Word], NativeScalar::I32),
    windows(FIRST + 144, b"msvcp140.dll", b"?sputc@?$basic_streambuf@DU?$char_traits@D@std@@@std@@QEAAHD@Z", Operation::Msvc(Msvc::Putc), &[Word, I32], NativeScalar::I32),
    windows(FIRST + 145, b"msvcp140.dll", b"?xsgetn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAA_JPEAD_J@Z", Operation::Msvc(Msvc::Getn), &[Word, Word, Word], NativeScalar::Word),
    windows(FIRST + 146, b"msvcp140.dll", b"?xsputn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MEAA_JPEBD_J@Z", Operation::Msvc(Msvc::Putn), &[Word, Word, Word], NativeScalar::Word),
    windows(FIRST + 147, b"msvcp140.dll", b"?sputn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@QEAA_JPEBD_J@Z", Operation::Msvc(Msvc::Sputn), &[Word, Word, Word], NativeScalar::Word),
    windows(FIRST + 148, b"msvcp140.dll", b"runtime:streambuf-seekoff", Operation::Msvc(Msvc::Seekoff), &[Word, Word, Word, I32, I32], NativeScalar::Word),
    windows(FIRST + 149, b"msvcp140.dll", b"runtime:streambuf-seekpos", Operation::Msvc(Msvc::Seekpos), &[Word, Word, Word, I32], NativeScalar::Word),
    windows(FIRST + 150, b"msvcp140.dll", b"??0?$basic_streambuf@DU?$char_traits@D@std@@@std@@IEAA@XZ", Operation::Msvc(Msvc::BufferCtor), &[Word], NativeScalar::Word),
    windows(FIRST + 151, b"msvcp140.dll", b"?flush@?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAAAEAV12@XZ", Operation::Msvc(Msvc::Flush), &[Word], NativeScalar::Word),
    windows(FIRST + 152, b"msvcp140.dll", b"?_Osfx@?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAAXXZ", Operation::Msvc(Msvc::Suffix), &[Word], NativeScalar::Void),
    windows(FIRST + 153, b"msvcp140.dll", b"?uncaught_exception@std@@YA_NXZ", Operation::Msvc(Msvc::Uncaught), &[], NativeScalar::U32),
    windows(FIRST + 154, b"msvcp140.dll", b"?tellp@?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAA?AV?$fpos@U_Mbstatet@@@2@XZ", Operation::Msvc(Msvc::Tellp), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 155, b"msvcp140.dll", b"??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAAAEAV01@H@Z", Operation::Msvc(Msvc::Integer32), &[Word, I32], NativeScalar::Word),
    windows(FIRST + 156, b"msvcp140.dll", b"??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAAAEAV01@_J@Z", Operation::Msvc(Msvc::Integer64), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 157, b"msvcp140.dll", b"??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QEAAAEAV01@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z", Operation::Msvc(Msvc::InsertBuffer), &[Word, Word], NativeScalar::Word),
    windows(FIRST + 200, b"msvcp140.dll", b"?_Id_cnt@id@locale@std@@0HA", Operation::Data(16), &[], Word),
    windows(FIRST + 201, b"msvcp140.dll", b"?id@?$numpunct@D@std@@2V0locale@2@A", Operation::Data(24), &[], Word),
    windows(FIRST + 300, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_configure_narrow_argv", Operation::Crt(Crt::Argv), &[I32], I32),
    windows(FIRST + 301, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_initialize_narrow_environment", Operation::Crt(Crt::Zero), &[], I32),
    windows(FIRST + 302, b"api-ms-win-crt-heap-l1-1-0.dll", b"_callnewh", Operation::Crt(Crt::Zero), &[Word], I32),
    windows(FIRST + 303, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_initialize_onexit_table", Operation::Crt(Crt::OnexitInit), &[Word], I32),
    windows(FIRST + 304, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_register_onexit_function", Operation::Crt(Crt::OnexitRegister), &[Word, Word], I32),
    windows(FIRST + 305, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_crt_atexit", Operation::Crt(Crt::Atexit), &[Word], I32),
    windows(FIRST + 306, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_execute_onexit_table", Operation::Crt(Crt::OnexitExecute), &[Word], I32),
    windows(FIRST + 307, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_cexit", Operation::Crt(Crt::Cexit), &[], Void),
    windows(FIRST + 308, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_initterm", Operation::Crt(Crt::InitTerm), &[Word, Word], Void),
    windows(FIRST + 309, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_initterm_e", Operation::Crt(Crt::InitTermError), &[Word, Word], I32),
    windows(FIRST + 310, b"vcruntime140.dll", b"__std_type_info_destroy_list", Operation::Crt(Crt::TypeInfo), &[Word], Void),
    windows(FIRST + 311, b"api-ms-win-crt-utility-l1-1-0.dll", b"qsort", Operation::Crt(Crt::Sort), &[Word, Word, Word, Word], Void),
    windows(FIRST + 312, b"api-ms-win-crt-math-l1-1-0.dll", b"acosf", Operation::Math(Math::Acos, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 313, b"api-ms-win-crt-math-l1-1-0.dll", b"sinf", Operation::Math(Math::Sin, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 314, b"api-ms-win-crt-math-l1-1-0.dll", b"ceilf", Operation::Math(Math::Ceil, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 315, b"api-ms-win-crt-math-l1-1-0.dll", b"cosf", Operation::Math(Math::Cos, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 316, b"api-ms-win-crt-math-l1-1-0.dll", b"truncf", Operation::Math(Math::Trunc, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 317, b"api-ms-win-crt-math-l1-1-0.dll", b"log2f", Operation::Math(Math::Log2, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 318, b"api-ms-win-crt-math-l1-1-0.dll", b"floorf", Operation::Math(Math::Floor, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 319, b"api-ms-win-crt-math-l1-1-0.dll", b"sqrtf", Operation::Math(Math::Sqrt, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 320, b"api-ms-win-crt-math-l1-1-0.dll", b"tanf", Operation::Math(Math::Tan, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 321, b"api-ms-win-crt-math-l1-1-0.dll", b"atan2f", Operation::Math(Math::Atan2, NativeScalar::Float), &[NativeScalar::Float, NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 322, b"api-ms-win-crt-math-l1-1-0.dll", b"fmodf", Operation::Math(Math::Fmod, NativeScalar::Float), &[NativeScalar::Float, NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 323, b"api-ms-win-crt-math-l1-1-0.dll", b"pow", Operation::Math(Math::Pow, Double), &[Double, Double], Double),
    windows(FIRST + 324, b"api-ms-win-crt-math-l1-1-0.dll", b"nextafterf", Operation::Math(Math::Nextafter, NativeScalar::Float), &[NativeScalar::Float, NativeScalar::Float], NativeScalar::Float),
    windows(FIRST + 325, b"api-ms-win-crt-math-l1-1-0.dll", b"modf", Operation::Math(Math::Modf, Double), &[Double, Word], Double),
    windows(FIRST + 326, b"api-ms-win-crt-math-l1-1-0.dll", b"_dclass", Operation::Math(Math::Class, Double), &[Double], NativeScalar::I16),
    windows(FIRST + 327, b"api-ms-win-crt-math-l1-1-0.dll", b"_fdclass", Operation::Math(Math::Class, NativeScalar::Float), &[NativeScalar::Float], NativeScalar::I16),
    windows(FIRST + 328, b"api-ms-win-crt-math-l1-1-0.dll", b"_dsign", Operation::Math(Math::Sign, Double), &[Double], NativeScalar::I16),
    windows(FIRST + 329, b"api-ms-win-crt-runtime-l1-1-0.dll", b"_errno", Operation::Crt(Crt::Errno), &[], Word),
    windows(FIRST + 330, b"api-ms-win-crt-locale-l1-1-0.dll", b"localeconv", Operation::Crt(Crt::Locale), &[], Word),
    windows(FIRST + 331, b"vcruntime140.dll", b"memchr", Operation::Find(false), &[Word, I32, Word], Word),
    windows(FIRST + 332, b"api-ms-win-crt-string-l1-1-0.dll", b"strncmp", Operation::Compare(Comparison::Prefix), &[Word, Word, Word], I32),
    windows(FIRST + 333, b"vcruntime140.dll", b"strchr", Operation::Find(true), &[Word, I32], Word),
    windows(FIRST + 334, b"vcruntime140.dll", b"strstr", Operation::Crt(Crt::Substring), &[Word, Word], Word),
    windows(FIRST + 335, b"api-ms-win-crt-convert-l1-1-0.dll", b"strtoul", Operation::Crt(Crt::Unsigned), &[Word, Word, I32], U32),
    windows(FIRST + 336, b"api-ms-win-crt-convert-l1-1-0.dll", b"atoi", Operation::Crt(Crt::Integer), &[Word], I32),
    windows(FIRST + 337, b"api-ms-win-crt-convert-l1-1-0.dll", b"atoll", Operation::Crt(Crt::Integer), &[Word], Word),
    windows(FIRST + 338, b"api-ms-win-crt-convert-l1-1-0.dll", b"atof", Operation::Crt(Crt::Float), &[Word], Double),
];

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Copy,
    Fill,
    Strncpy,
    Length,
    Compare(Comparison),
    Find(bool),
    Math(Math, NativeScalar),
    Crt(Crt),
    Malloc,
    Calloc,
    Realloc,
    Free,
    Msvc(Msvc),
    Data(usize),
}

#[derive(Clone, Copy)]
pub(crate) enum Math {
    Sin,
    Cos,
    Atan2,
    Sqrt,
    Floor,
    Ceil,
    Acos,
    Absolute,
    Trunc,
    Log2,
    Tan,
    Fmod,
    Pow,
    Nextafter,
    Modf,
    Class,
    Sign,
}
#[derive(Clone, Copy)]
pub(crate) enum Crt {
    Argv,
    Zero,
    OnexitInit,
    OnexitRegister,
    Atexit,
    OnexitExecute,
    Cexit,
    InitTerm,
    InitTermError,
    TypeInfo,
    Sort,
    Errno,
    Locale,
    Substring,
    Unsigned,
    Integer,
    Float,
}
#[derive(Clone, Copy)]
pub(crate) enum Comparison {
    String,
    Memory,
    Prefix,
}

pub fn function(number: u32) -> Option<&'static Function> {
    FUNCTIONS.iter().find(|f| f.number == number)
}
impl Function {
    pub fn windows_provider(&self, name: &[u8]) -> bool {
        use qa_core::names::compare_folded;
        let equal = |expected| compare_folded(name, expected).is_eq();
        if equal(b"msvcrt.dll") || equal(b"ucrtbase.dll") {
            return self.provider != b"msvcp140.dll";
        }
        if self.provider.ends_with(b".dll") {
            return equal(self.provider);
        }
        let library = match self.operation {
            Operation::Copy | Operation::Fill | Operation::Compare(Comparison::Memory) => {
                if equal(b"vcruntime140.dll") || equal(b"api-ms-win-crt-memory-l1-1-0.dll") {
                    return true;
                }
                b"api-ms-win-crt-string-l1-1-0.dll".as_slice()
            }
            Operation::Strncpy | Operation::Length | Operation::Compare(Comparison::String) => {
                b"api-ms-win-crt-string-l1-1-0.dll"
            }
            Operation::Malloc | Operation::Calloc | Operation::Free | Operation::Realloc => {
                b"api-ms-win-crt-heap-l1-1-0.dll"
            }
            Operation::Math(_, _) => b"api-ms-win-crt-math-l1-1-0.dll",
            _ => return false,
        };
        equal(library)
    }
    pub fn data_offset(&self) -> Option<usize> {
        if let Operation::Data(offset) = self.operation {
            Some(offset)
        } else {
            None
        }
    }
    pub fn windows_object(&self) -> bool {
        matches!(self.operation, Operation::Msvc(_))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub base: u64,
    pub heap_bytes: usize,
    pub teb: Option<u64>,
}
// TEB, PEB, static TLS vector and expansion slots are distinct owned storage,
// following the C runtime's x64 layout. The module TLS template follows them.
pub const THREAD_BYTES: usize = 0x8000;
pub const STATIC_TLS_OFFSET: usize = 0x3000;
pub const TLS_DATA_OFFSET: usize = 0x7000;
impl RuntimeConfig {
    pub fn prepare_crt(self, page: &mut [u8]) -> Result<(), super::NativeError> {
        let page = page.get_mut(..608).ok_or(super::NativeError::Extent)?;
        // C lconv strings and fields, owned once at load. The remainder of the
        // initially zeroed page holds the onexit table and errno.
        for index in 0..10 {
            let value = self.base + if index == 0 { 604 } else { 600 };
            page[416 + index * 8..424 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        page[496..512].fill(127);
        page[604..606].copy_from_slice(b".\0");
        Ok(())
    }
    /// Exact x64 MSVC object/vtable layout from the C runtime. The one page is
    /// owned writable module storage; callable slots refer to child gateways.
    pub fn prepare_msvc(
        self,
        page: &mut [u8],
        target: impl Fn(u32) -> Result<u64, super::NativeError>,
    ) -> Result<(), super::NativeError> {
        let mut put = |offset: usize, bytes: &[u8]| {
            let end = offset
                .checked_add(bytes.len())
                .ok_or(super::NativeError::Extent)?;
            page.get_mut(offset..end)
                .ok_or(super::NativeError::Extent)?
                .copy_from_slice(bytes);
            Ok::<_, super::NativeError>(())
        };
        for (offset, data) in [
            (160, &b"C\0"[..]),
            (164, &b"true\0"[..]),
            (172, &b"false\0"[..]),
        ] {
            put(offset, data)?;
        }
        for (offset, value) in [(96, self.base + 208), (136, self.base + 160)] {
            put(offset, &value.to_le_bytes())?;
        }
        put(104, &2u32.to_le_bytes())?;
        put(128, &63u32.to_le_bytes())?;
        for (offset, numbers) in [
            (208, &[103, 100, 101][..]),
            (240, &[116][..]),
            (248, &[125][..]),
            (256, &[125][..]),
            (
                264,
                &[
                    127, 128, 129, 130, 131, 133, 132, 143, 145, 146, 148, 149, 135, 134, 136,
                ][..],
            ),
        ] {
            for (index, number) in numbers.iter().enumerate() {
                put(offset + index * 8, &target(FIRST + number)?.to_le_bytes())?;
            }
        }
        put(388, &16u32.to_le_bytes())?;
        put(396, &32u32.to_le_bytes())?;
        put(404, &16u32.to_le_bytes())?;
        drop(put);
        self.prepare_crt(page)
    }
}

const fn windows(
    number: u32,
    library: &'static [u8],
    name: &'static [u8],
    operation: Operation,
    parameters: &'static [NativeScalar],
    result: NativeScalar,
) -> Function {
    Function {
        name,
        number,
        heap: true,
        provider: library,
        versions: &[],
        parameters,
        result,
        operation,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Msvc {
    Incref,
    Decref,
    FacetDtor,
    FacetDelete,
    FacetCtor,
    LocaleInit,
    Global,
    Lock,
    Unlock,
    LocinfoCtor,
    LocinfoDtor,
    True,
    False,
    Lconv,
    Cvtvec,
    IosDtor,
    IosDelete,
    IosCtor,
    Rdbuf,
    Setstate,
    Good,
    OstreamCtor,
    IostreamCtor,
    OstreamDtor,
    IostreamDtor,
    OstreamDelete,
    BufferDtor,
    BufferDelete,
    BufferLock,
    BufferOverflow,
    BufferUnderflow,
    Showmany,
    Sync,
    Setbuf,
    Imbue,
    Eback,
    Pbase,
    Gptr,
    Pptr,
    Egptr,
    Epptr,
    Uflow,
    Putc,
    Getn,
    Putn,
    Sputn,
    Seekoff,
    Seekpos,
    BufferCtor,
    Flush,
    Suffix,
    Uncaught,
    Tellp,
    Integer32,
    Integer64,
    InsertBuffer,
}
