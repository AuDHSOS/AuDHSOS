// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![forbid(unsafe_code)]
fuzz_support::fuzz_target!(|bytes:&[u8]|{
    let mut data=[0u8;16];let n=bytes.len().min(16);data[..n].copy_from_slice(&bytes[..n]);
    let x=f64::from_bits(u64::from_le_bytes(data[..8].try_into().unwrap()));
    let y=f64::from_bits(u64::from_le_bytes(data[8..].try_into().unwrap()));
    check(x,y);
    let mantissa=f64::from_bits((x.to_bits()&0x000f_ffff_ffff_ffff)|0x3ff0_0000_0000_0000);
    let exponent=f64::from(i16::from_le_bytes([data[8],data[9]]))/16.0;
    check(mantissa,exponent);
    check(f64::from_bits(0x3ff0_0000_0000_0000+u64::from(data[0])),exponent*1e13);
    check(f64::from_bits(0x3ff0_0000_0000_0000-u64::from(data[1])),exponent*1e13);
    integer(bytes);
});
fn integer(bytes: &[u8]) {
    let radix=2+u32::from(bytes.first().copied().unwrap_or(0)%35);
    let mut n=audhsos_math::RadixInteger::new(radix).unwrap();
    let mut exact=Some(0u128);
    for b in bytes.iter().take(4096) {
        let digit=u32::from(*b)%radix;
        assert!(n.push(digit));
        exact=exact.and_then(|v| v.checked_mul(u128::from(radix))?.checked_add(u128::from(digit)));
        if let Some(exact)=exact { assert_eq!(n.to_f64().to_bits(),exact.to_string().parse::<f64>().unwrap().to_bits()); }
    }
    assert!(!n.push(radix));
    let decimal: String=bytes.iter().take(4096).map(|b| char::from(b'0'+b%10)).collect();
    let mut n=audhsos_math::RadixInteger::new(10).unwrap();
    for b in decimal.bytes() { assert!(n.push(u32::from(b-b'0'))); }
    if !decimal.is_empty() { assert_eq!(n.to_f64().to_bits(),decimal.parse::<f64>().unwrap().to_bits()); }
}
fn check(x:f64,y:f64){
    let result=audhsos_math::pow(x,y);
    let reference=if y.is_nan()||x.abs()==1.0&&y.is_infinite(){f64::NAN}else{x.powf(y)};
    if reference.is_nan(){assert!(result.is_nan(),"{x:?} ** {y:?}: {result:?}");}
    else{assert!(result.to_bits().abs_diff(reference.to_bits())<=4,"{x:?} ** {y:?}: {result:?} vs {reference:?}");}
}
