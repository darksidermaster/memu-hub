// DPAPI (CurrentUser): only the same Windows user can decrypt this Registry value.
#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, ptr};
    use winreg::{enums::*, RegKey, RegValue};
    #[repr(C)]
    struct Blob { len: u32, data: *mut u8 }
    #[link(name="crypt32")]
    extern "system" {
        fn CryptProtectData(input: *const Blob, description: *const u16, entropy: *const Blob,
            reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
        fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob,
            reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
    }
    #[link(name="kernel32")]
    extern "system" { fn LocalFree(mem: *mut c_void) -> *mut c_void; }
    const PATH: &str = r"Software\MEmuHub";
    fn crypt(bytes: &[u8], protect: bool) -> Result<Vec<u8>, String> {
        let input = Blob { len: bytes.len().try_into().map_err(|_| "Dados muito grandes")?, data: bytes.as_ptr() as *mut u8 };
        let mut out = Blob { len: 0, data: ptr::null_mut() };
        let ok = unsafe {
            if protect { CryptProtectData(&input, ptr::null(), ptr::null(), ptr::null_mut(), ptr::null_mut(), 1, &mut out) }
            else { CryptUnprotectData(&input, ptr::null_mut(), ptr::null(), ptr::null_mut(), ptr::null_mut(), 1, &mut out) }
        };
        if ok == 0 { return Err("Não foi possível acessar os dados protegidos do Windows.".into()); }
        let result = unsafe { std::slice::from_raw_parts(out.data, out.len as usize).to_vec() };
        unsafe { LocalFree(out.data.cast()); }
        Ok(result)
    }
    pub fn read() -> Result<Option<Vec<u8>>, String> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let key = match root.open_subkey(PATH) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Não foi possível ler o Registro.".into()),
        };
        match key.get_raw_value("Vault") {
            Ok(v) => crypt(&v.bytes, false).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("Não foi possível ler a configuração.".into()),
        }
    }
    pub fn write(bytes: &[u8]) -> Result<(), String> {
        let protected = crypt(bytes, true)?;
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(PATH).map_err(|_| "Não foi possível abrir o Registro.")?;
        key.set_raw_value("Vault", &RegValue { bytes: protected, vtype: REG_BINARY }).map_err(|_| "Não foi possível salvar no Registro.".into())
    }
}
#[cfg(not(windows))]
mod platform {
    pub fn read() -> Result<Option<Vec<u8>>, String> { Err("Este aplicativo requer Windows.".into()) }
    pub fn write(_: &[u8]) -> Result<(), String> { Err("Este aplicativo requer Windows.".into()) }
}
pub use platform::{read, write};
