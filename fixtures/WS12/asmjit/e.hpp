// This file is part of AsmJit project <https://asmjit.com>
//
// See <asmjit/core.h> or LICENSE.md for license and copyright information
// SPDX-License-Identifier: Zlib

#ifndef ASMJIT_X86_X86EMITTER_H_INCLUDED
#define ASMJIT_X86_X86EMITTER_H_INCLUDED

#include <asmjit/core/emitter.h>
#include <asmjit/support/support.h>
#include <asmjit/x86/x86globals.h>
#include <asmjit/x86/x86operand.h>

ASMJIT_BEGIN_SUB_NAMESPACE(x86)

#define ASMJIT_INST_0x(NAME, ID) \
  inline Error NAME() { return _emitter()->_emitI(Inst::kId##ID); }

#define ASMJIT_INST_1x(NAME, ID, T0) \
  inline Error NAME(const T0& o0) { return _emitter()->_emitI(Inst::kId##ID, o0); }
#define ASMJIT_INST_6x(NAME, ID, T0, T1, T2, T3, T4, T5) \
  inline Error NAME(const T0& o0, const T1& o1, const T2& o2, const T3& o3, const T4& o4, const T5& o5) { return _emitter()->_emitI(Inst::kId##ID, o0, o1, o2, o3, o4, o5); }

//! \addtogroup asmjit_x86
//! \{

//! Emitter (X86 - explicit).
template<typename This>
struct EmitterExplicitT {
  //! \cond

  // These types are used to describe implicit operands passed explicitly.
  using Gp_AL = Gp;
  using Gp_AH = Gp;
  using Gp_CL = Gp;
  using Gp_AX = Gp;
  using Gp_DX = Gp;

  using Gp_EAX = Gp;
  using Gp_EBX = Gp;
  using Gp_ECX = Gp;
  using Gp_EDX = Gp;

  using Gp_RAX = Gp;
  using Gp_RBX = Gp;
  using Gp_RCX = Gp;
  using Gp_RDX = Gp;

  using Gp_ZAX = Gp;
  using Gp_ZBX = Gp;
  using Gp_ZCX = Gp;
  using Gp_ZDX = Gp;

  using DS_ZAX = Mem; // ds:[zax]
  using DS_ZDI = Mem; // ds:[zdi]
  using ES_ZDI = Mem; // es:[zdi]
  using DS_ZSI = Mem; // ds:[zsi]

  using XMM0 = Vec;

  // These two are unfortunately reported by the sanitizer. We know what we do, however, the sanitizer doesn't.
  // I have tried to use reinterpret_cast instead, but that would generate bad code when compiled by MSC.
  ASMJIT_ATTRIBUTE_NO_SANITIZE_UNDEF ASMJIT_INLINE_NODEBUG This* _emitter() noexcept { return static_cast<This*>(this); }
  ASMJIT_ATTRIBUTE_NO_SANITIZE_UNDEF ASMJIT_INLINE_NODEBUG const This* _emitter() const noexcept { return static_cast<const This*>(this); }

  //! \endcond

  //! \name Native Registers
  //! \{

  //! Returns either 32-bit or 64-bit GP register of the given `id` depending on the emitter's architecture.
  ASMJIT_INST_2x(lea, Lea, Gp, Mem)
};
ASMJIT_END_SUB_NAMESPACE
