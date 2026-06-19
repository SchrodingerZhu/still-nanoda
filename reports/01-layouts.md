# lean_object layouts: Declaration & ConstantInfo

Source: `lean4/src/kernel/declaration.h` (C++ `cnstr_get_ref` slots) + `src/kernel/expr.h`.
All object fields are `lean_object*` at slot i (`ctor_get(o,i)`); scalars follow the
object fields. Tag = `ptr_tag(o)` (`cnstr_tag`).

## Declaration (tag)
Axiom=0, Definition=1, Theorem=2, Opaque=3, Quot=4, MutualDefinition=5, Inductive=6
- Axiom/Def/Thm/Opaque/Mutual: slot0 = the *Val (or `List DefinitionVal` for Mutual)
- Quot=4: nullary scalar `box(4)`
- Inductive=6: slot0=lparams(`List Name`), slot1=nparams(Nat), slot2=types(`List InductiveType`), isUnsafe scalar

## ConstantInfo (tag)
Axiom=0, Definition=1, Theorem=2, Opaque=3, Quot=4, Inductive=5, Constructor=6, Recursor=7
- slot0 = the *Val

## ConstantVal
slot0=name(Name), slot1=levelParams(`List Name`), slot2=type(Expr)

## *Val (slot0 = ConstantVal in all of them)
- AxiomVal: slot0=CV; isUnsafe scalar
- DefinitionVal: slot0=CV, slot1=value(Expr), slot2=hints(ReducibilityHints); safety scalar (+ all)
- TheoremVal: slot0=CV, slot1=value(Expr) (+ all)
- OpaqueVal: slot0=CV, slot1=value(Expr); isUnsafe scalar (+ all)
- QuotVal: slot0=CV; kind scalar (QuotKind type=0,ctor=1,lift=2,ind=3)
- InductiveVal: slot0=CV, slot1=numParams(Nat), slot2=numIndices(Nat), slot3=all(`List Name`),
  slot4=ctors(`List Name`), slot5=numNested(Nat); isRec, isUnsafe, isReflexive scalars
- ConstructorVal: slot0=CV, slot1=induct(Name), slot2=cidx(Nat), slot3=numParams(Nat),
  slot4=numFields(Nat); isUnsafe scalar
- RecursorVal: slot0=CV, slot1=all(`List Name`), slot2=numParams, slot3=numIndices,
  slot4=numMotives, slot5=numMinors, slot6=rules(`List RecursorRule`); k, isUnsafe scalars

## Aux
- RecursorRule: slot0=ctor(Name), slot1=nfields(Nat), slot2=rhs(Expr)
- InductiveType: slot0=name(Name), slot1=type(Expr), slot2=ctors(`List Constructor`)
- Constructor: pair_ref<Name,Expr> -> slot0=name, slot1=type
- ReducibilityHints: tag 0=Opaque, 1=Abbreviation, 2=Regular(u32 scalar)
- List T: `nil` scalar, `cons` tag-1 slot0=head slot1=tail

## Mapping to sokonanoda `Declar` (src/env.rs)
- DeclarInfo { name, uparams: LevelsPtr, ty }. NOTE uparams is a list of **Level::Param**
  (one per levelParam Name), not the raw names -> convert each Name -> param(name).
- Axiom{info}, Definition{info,val,hint}, Theorem{info,val}, Opaque{info,val}, Quot{info}
- Inductive(InductiveData{info,is_recursive,is_nested,num_params,num_indices,all_ind_names,all_ctor_names})
- Constructor(ConstructorData{info,inductive_name,ctor_idx,num_params,num_fields})
- Recursor(RecursorData{info,all_inductives,num_params,num_indices,num_motives,num_minors,rec_rules,is_k})
- RecRule{ctor_name, ctor_telescope_size_wo_params (=nfields), val (=rhs)}
- ReducibilityHint: Opaque|Regular(u16)|Abbrev  <- Lean Opaque/Regular(u32)/Abbreviation

## Host callbacks needed (external_checker.h host table)
1. tick; 2. mk_ok/mk_error/mk_kernel_exception; 3. find_const(env,name)->Option ConstantInfo;
4. builtin_add_decl (check+add, for delegated inductive/quot/mutual);
5. builtin_add_unchecked (add only, after sokonanoda checks a def/thm) -- TO ADD.
