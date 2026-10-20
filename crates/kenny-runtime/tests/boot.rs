use kenny_runtime::core::*;

#[test]
fn boots_and_dispatches() {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            init_runtime();
            let r = send(rt().main_obj, sym("puts"), &[mkfix(42)], QNIL);
            assert_ne!(r, UNDEF);
            let s = send(mkfix(7), sym("+"), &[mkfix(5)], QNIL);
            assert_eq!(s, mkfix(12));
            kenny_runtime::kernel::flush_out();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn object_sizes() {
    use std::mem::size_of;
    eprintln!(
        "RObject {} RString {} RArray {} RHash {} RFloat {} RBignum {} RRange {} RProc {} RData {}",
        size_of::<RObject>(), size_of::<RString>(), size_of::<RArray>(), size_of::<RHash>(),
        size_of::<RFloat>(), size_of::<RBignum>(), size_of::<RRange>(), size_of::<RProc>(), size_of::<RData>()
    );
}
