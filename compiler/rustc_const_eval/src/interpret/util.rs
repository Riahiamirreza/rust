use rustc_middle::mir::interpret::InterpResult;
use rustc_middle::ty::{
    self, Ty, TyCtxt, TypeSuperVisitable, TypeVisitable, TypeVisitableExt, TypeVisitor,
};
use std::ops::ControlFlow;

/// Checks whether a type contains generic parameters which must be instantiated.
///
/// In case it does, returns a `TooGeneric` const eval error. Note that due to polymorphization
/// types may be "concrete enough" even though they still contain generic parameters in
/// case these parameters are unused.
pub(crate) fn ensure_monomorphic_enough<'tcx, T>(tcx: TyCtxt<'tcx>, ty: T) -> InterpResult<'tcx>
where
    T: TypeVisitable<TyCtxt<'tcx>>,
{
    debug!("ensure_monomorphic_enough: ty={:?}", ty);
    if !ty.has_param() {
        return Ok(());
    }

    struct FoundParam;
    struct UsedParamsNeedInstantiationVisitor<'tcx> {
        tcx: TyCtxt<'tcx>,
    }

    impl<'tcx> TypeVisitor<TyCtxt<'tcx>> for UsedParamsNeedInstantiationVisitor<'tcx> {
        type BreakTy = FoundParam;

        fn visit_ty(&mut self, ty: Ty<'tcx>) -> ControlFlow<Self::BreakTy> {
            if !ty.has_param() {
                return ControlFlow::Continue(());
            }

            match *ty.kind() {
                ty::Param(_) => ControlFlow::Break(FoundParam),
                ty::Closure(def_id, args)
                | ty::CoroutineClosure(def_id, args, ..)
                | ty::Coroutine(def_id, args, ..)
                | ty::FnDef(def_id, args) => {
                    let instance = ty::InstanceDef::Item(def_id);
                    let unused_params = self.tcx.unused_generic_params(instance);
                    for (index, arg) in args.into_iter().enumerate() {
                        let index = index
                            .try_into()
                            .expect("more generic parameters than can fit into a `u32`");
                        // Only recurse when generic parameters in fns, closures and coroutines
                        // are used and have to be instantiated.
                        //
                        // Just in case there are closures or coroutines within this arg,
                        // recurse.
                        if unused_params.is_used(index) && arg.has_param() {
                            return arg.visit_with(self);
                        }
                    }
                    ControlFlow::Continue(())
                }
                _ => ty.super_visit_with(self),
            }
        }

        fn visit_const(&mut self, c: ty::Const<'tcx>) -> ControlFlow<Self::BreakTy> {
            match c.kind() {
                ty::ConstKind::Param(..) => ControlFlow::Break(FoundParam),
                _ => c.super_visit_with(self),
            }
        }
    }

    let mut vis = UsedParamsNeedInstantiationVisitor { tcx };
    if matches!(ty.visit_with(&mut vis), ControlFlow::Break(FoundParam)) {
        throw_inval!(TooGeneric);
    } else {
        Ok(())
    }
}
