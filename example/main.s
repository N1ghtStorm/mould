.build_version macos, 11, 0
.section __TEXT,__cstring,cstring_literals
L_mould_fmt_signed:
    .asciz "%ld\n"
L_mould_fmt_unsigned:
    .asciz "%lu\n"
L_mould_fmt_float:
    .asciz "%f\n"
L_mould_fmt_pointer:
    .asciz "%p\n"
L_mould_bool_true:
    .asciz "true"
L_mould_bool_false:
    .asciz "false"

.section __TEXT,__text,regular,pure_instructions

.globl _mould_sum
.p2align 2
_mould_sum:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    sub sp, sp, #16
    sub x9, x29, #8
    str x0, [x9]
    sub x9, x29, #16
    str x1, [x9]
    sub x9, x29, #8
    ldr x0, [x9]
    sxtw x0, w0
    sub sp, sp, #16
    str x0, [sp]
    sub x9, x29, #16
    ldr x0, [x9]
    sxtw x0, w0
    mov x1, x0
    ldr x0, [sp]
    add sp, sp, #16
    add x0, x0, x1
    sxtw x0, w0
    b L_mould_sum_return
L_mould_sum_return:
    add sp, sp, #16
    ldp x29, x30, [sp], #16
    ret

.globl _main
.p2align 2
_main:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    sub sp, sp, #48
    movz x0, #9
    sxtw x0, w0
    sub x9, x29, #8
    str x0, [x9]
    movz x0, #7
    sxtw x0, w0
    sub x9, x29, #16
    str x0, [x9]
    sub x0, x29, #16
    add x0, x0, #8
    ldr x0, [x0]
    sxtw x0, w0
    sub sp, sp, #16
    str x0, [sp]
    sub x0, x29, #16
    ldr x0, [x0]
    sxtw x0, w0
    sub sp, sp, #16
    str x0, [sp]
    ldr x0, [sp, #0]
    ldr x1, [sp, #16]
    add sp, sp, #32
    bl _mould_sum
    sxtw x0, w0
    sub x9, x29, #24
    str x0, [x9]
    movz x0, #8
    bl _malloc
    sub x9, x29, #40
    str x0, [x9]
    sub x9, x29, #24
    ldr x0, [x9]
    sxtw x0, w0
    sub x9, x29, #40
    ldr x9, [x9]
    str x0, [x9]
    sub x9, x29, #40
    ldr x0, [x9]
    sub x9, x29, #32
    str x0, [x9]
    sub x9, x29, #32
    ldr x0, [x9]
    ldr x0, [x0]
    sxtw x0, w0
    sub sp, sp, #16
    str x0, [sp]
    movz x0, #16
    sxtw x0, w0
    mov x1, x0
    ldr x0, [sp]
    add sp, sp, #16
    cmp x0, x1
    cset x0, eq
    cmp x0, #0
    b.eq L_mould_if_else_0
    sub x9, x29, #32
    ldr x0, [x9]
    ldr x0, [x0]
    sxtw x0, w0
    mov x1, x0
    sub sp, sp, #16
    str x1, [sp]
    adrp x0, L_mould_fmt_signed@PAGE
    add x0, x0, L_mould_fmt_signed@PAGEOFF
    bl _printf
    add sp, sp, #16
    b L_mould_if_end_1
L_mould_if_else_0:
    movz x0, #0
    sxtw x0, w0
    mov x1, x0
    sub sp, sp, #16
    str x1, [sp]
    adrp x0, L_mould_fmt_signed@PAGE
    add x0, x0, L_mould_fmt_signed@PAGEOFF
    bl _printf
    add sp, sp, #16
L_mould_if_end_1:
    sub x9, x29, #32
    ldr x0, [x9]
    bl _free
L_mould_main_return:
    mov x0, #0
    add sp, sp, #48
    ldp x29, x30, [sp], #16
    ret


.globl _mould_print_i128
.p2align 2
_mould_print_i128:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    tbz x1, #63, L_mould_print_i128_positive
    mvn x0, x0
    mvn x1, x1
    adds x0, x0, #1
    adc x1, x1, xzr
    sub sp, sp, #16
    stp x0, x1, [sp]
    mov x0, #45
    bl _putchar
    ldp x0, x1, [sp]
    add sp, sp, #16
L_mould_print_i128_positive:
    bl _mould_print_u128
    ldp x29, x30, [sp], #16
    ret

.globl _mould_print_u128
.p2align 2
_mould_print_u128:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    stp x19, x20, [sp, #-16]!
    sub sp, sp, #64
    add x19, sp, #63
    strb wzr, [x19]
    cmp x0, #0
    ccmp x1, #0, #0, eq
    b.ne L_mould_print_u128_loop
    sub x19, x19, #1
    mov w6, #48
    strb w6, [x19]
    b L_mould_print_u128_emit
L_mould_print_u128_loop:
    bl L_mould_udivmod10_u128
    sub x19, x19, #1
    add w6, w2, #48
    strb w6, [x19]
    cmp x0, #0
    ccmp x1, #0, #0, eq
    b.ne L_mould_print_u128_loop
L_mould_print_u128_emit:
    mov x0, x19
    bl _puts
    add sp, sp, #64
    ldp x19, x20, [sp], #16
    ldp x29, x30, [sp], #16
    ret

.p2align 2
L_mould_udivmod10_u128:
    mov x6, x0
    mov x7, x1
    mov x0, #0
    mov x1, #0
    mov x2, #0
    mov x5, #128
L_mould_udivmod10_u128_loop:
    lsr x8, x7, #63
    lsl x2, x2, #1
    orr x2, x2, x8
    cmp x2, #10
    b.lo L_mould_udivmod10_u128_no_sub
    sub x2, x2, #10
    mov x8, #1
    b L_mould_udivmod10_u128_have_bit
L_mould_udivmod10_u128_no_sub:
    mov x8, #0
L_mould_udivmod10_u128_have_bit:
    adds x0, x0, x0
    adcs x1, x1, x1
    orr x0, x0, x8
    adds x6, x6, x6
    adc x7, x7, x7
    subs x5, x5, #1
    b.ne L_mould_udivmod10_u128_loop
    ret

.p2align 2
L_mould_udivmod_u128:
    cmp x2, #0
    ccmp x3, #0, #0, eq
    b.ne L_mould_udivmod_u128_nonzero
    mov x0, #0
    mov x1, #0
    mov x2, #0
    mov x3, #0
    ret
L_mould_udivmod_u128_nonzero:
    mov x6, x0
    mov x7, x1
    mov x0, #0
    mov x1, #0
    mov x4, #0
    mov x5, #0
    mov x9, #128
L_mould_udivmod_u128_loop:
    lsr x8, x7, #63
    adds x4, x4, x4
    adcs x5, x5, x5
    orr x4, x4, x8
    cmp x5, x3
    b.hi L_mould_udivmod_u128_sub
    b.lo L_mould_udivmod_u128_no_sub
    cmp x4, x2
    b.lo L_mould_udivmod_u128_no_sub
L_mould_udivmod_u128_sub:
    subs x4, x4, x2
    sbc x5, x5, x3
    mov x8, #1
    b L_mould_udivmod_u128_have_bit
L_mould_udivmod_u128_no_sub:
    mov x8, #0
L_mould_udivmod_u128_have_bit:
    adds x0, x0, x0
    adcs x1, x1, x1
    orr x0, x0, x8
    adds x6, x6, x6
    adc x7, x7, x7
    subs x9, x9, #1
    b.ne L_mould_udivmod_u128_loop
    mov x2, x4
    mov x3, x5
    ret

.subsections_via_symbols
