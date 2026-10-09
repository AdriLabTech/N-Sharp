> Copia literal de la especificación N# v0.1 (nota `N Sharp - Especificacion v0.1` del vault de Adrián).
> Al exportarla de Obsidian los bloques de código perdieron sus vallas ```; las líneas `text`, `nsharp`, `toml` y `ebnf`
> que preceden a un bloque indican el lenguaje del fragmento que sigue.
> Las decisiones de implementación están en [decisiones.md](decisiones.md).

# N# Language Specification v0.1

## Tabla de contenidos

1. Introducción
    
2. Filosofía y objetivos
    
3. Modelo de compilación
    
4. Estructura léxica
    
5. Identificadores
    
6. Palabras reservadas
    
7. Comentarios
    
8. Literales
    
9. Tipos
    
10. Variables y declaraciones
    
11. Expresiones
    
12. Operadores y precedencia
    
13. Control de flujo
    
14. Funciones
    
15. Closures y function types
    
16. Arrays
    
17. Collections
    
18. Tuplas
    
19. Structs
    
20. Impl blocks
    
21. Traits
    
22. Enums
    
23. Pattern matching
    
24. Generics
    
25. Type aliases
    
26. Union types
    
27. Option
    
28. Result
    
29. Error handling
    
30. Memoria y RAII
    
31. References
    
32. Pointers
    
33. unsafe
    
34. const
    
35. static
    
36. FFI
    
37. Inline assembly
    
38. Macros
    
39. Attributes
    
40. Modules
    
41. Packages
    
42. Standard Library
    
43. Threads
    
44. Channels
    
45. Async/Await
    
46. Testing
    
47. Documentation
    
48. Formatter
    
49. Compiler diagnostics
    
50. Project structure
    
51. nsharp.toml
    
52. ncom CLI
    
53. Script mode
    
54. Project mode
    
55. Compile-time semantics
    
56. Runtime semantics
    
57. ABI
    
58. Formal grammar
    
59. Reserved/future features
    
60. Appendix: ejemplos completos
    

---

## 1. Introducción

N# (N Sharp) es un lenguaje de programación compilado, estáticamente tipado y fuertemente tipado, diseñado para combinar la claridad estructural de Java, la seguridad y expresividad de Rust, el control y rendimiento de C, y la legibilidad y productividad de Python.

N# es un lenguaje **generalista con capacidad systems**. DEBE servir para aplicaciones de propósito general, backend, CLI, herramientas de desarrollo, aplicaciones de alto rendimiento, programación de sistemas, interoperabilidad con C y, eventualmente, desarrollo de software de bajo nivel.

N# se compila a código nativo mediante **NCompiler**. Su CLI oficial es **ncom**. Los archivos fuente utilizan la extensión **.ncs**.

Esta especificación es normativa. Utiliza los siguientes términos:

- **DEBE**: requisito obligatorio.
    
- **NO DEBE**: prohibición.
    
- **PUEDE**: comportamiento permitido/opcional.
    
- **DEBERÍA**: recomendación.
    
- **NO DEBERÍA**: recomendación negativa.
    
- **ERROR DE COMPILACIÓN**: construcción rechazada durante compilación.
    
- **UNDEFINED / UNSPECIFIED**: comportamiento no definido por esta versión.
    

---

## 2. Filosofía y objetivos

N# se define como:

> Lenguaje generalista compilado a código nativo, estáticamente tipado, fuertemente tipado, con inferencia local, RAII, traits, pattern matching y mecanismos explícitos `unsafe` para programación de sistemas.

Filosofía: **Performance + Readability**.

Objetivos secundarios: productividad y control.

N# DEBE resultar familiar a programadores de Java, Rust, C y Python, pero NO DEBE presentarse como un dialecto de ninguno de ellos. N# posee una identidad sintáctica propia.

---

## 3. Modelo de compilación

N# se compila a código nativo mediante NCompiler. El pipeline de compilación DEBE incluir conceptualmente:

text

Fuente .ncs
    ↓
Lexer
    ↓
Parser
    ↓
AST
    ↓
Name Resolution
    ↓
Type Checker
    ↓
Semantic Analysis
    ↓
N# IR
    ↓
Native Code Generation
    ↓
Runtime

La representación intermedia (N# IR), el backend de código nativo, el runtime, el ABI concreto, la calling convention y el linker behavior se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION** en esta especificación, salvo donde se indique explícitamente lo contrario.

---

## 4. Estructura léxica

N# es case-sensitive. `name`, `Name` y `NAME` son tres identificadores distintos.

Los archivos fuente DEBEN estar codificados en UTF-8.

El lexer DEBE reconocer:

- identificadores;
    
- palabras reservadas;
    
- literales;
    
- operadores;
    
- delimitadores;
    
- comentarios;
    
- documentación.
    

El lexer NO DEBE utilizar indentación significativa.

---

## 5. Identificadores

### Sintaxis

Un identificador DEBE comenzar por una letra Unicode o por `_`, seguido de letras, dígitos o `_`.

text

identifier := (letter | "_") (letter | digit | "_")*

### Semántica estática

- Los identificadores PUEDEN contener `_`.
    
- Los identificadores PUEDEN contener números.
    
- Los identificadores PUEDEN comenzar por `_`.
    
- Las secuencias compuestas únicamente por `_` (`_`, `__`, `___`, ...) están reservadas y NO son identificadores válidos.
    
- `_` se utiliza exclusivamente como wildcard.
    
- `$` está reservado para la sintaxis de tipos primitivos y NO PUEDE formar parte de identificadores.
    

### Ejemplos válidos

nsharp

let name $string = "Adrian";
let _internal $int = 10;
let user2 $User;
let __value $float = 3.14;

### Ejemplos inválidos

nsharp

let _ $int = 10;        # ERROR: _ es wildcard reservado
let $var $int = 10;     # ERROR: $ reservado para tipos

---

## 6. Palabras reservadas

Las palabras reservadas de N# v0.1 son exactamente:

text

fn
if
else
loop
while
break
return
struct
enum
trait
impl
match
import
pub
const
static
true
false
null
nothing
unsafe
async
await
self
Self
as
type
test
assert
exit
panic
let

### Notas normativas

- `new` NO es keyword. `User.new(...)` es una llamada a método estático ordinaria.
    
- `Object` NO es keyword ni tipo especial.
    
- `continue` NO es keyword en v0.1.
    
- `switch` NO es keyword en v0.1.
    

---

## 7. Comentarios

### Comentario de línea

nsharp

# comentario

### Comentario multilínea

nsharp

!#
    comentario
    multilínea
#!

Los comentarios multilínea PUEDEN anidarse. El lexer DEBE implementar un contador de profundidad:

nsharp

!#
    texto
    !#
        comentario interno
    #!
    más texto
#!

### Documentación

nsharp

/// Esta función suma dos números.
fn add(a $int, b $int) -> $int {
    return a + b;
}

`///` es documentación oficial y es utilizada por `ncom doc`.

---

## 8. Literales

### Enteros

nsharp

42
0xFF
0b101010
1_000_000

NO existen literales octales en v0.1.

### Floats

nsharp

3.14

NO existe notación científica en v0.1.

### Booleanos

nsharp

true
false

### Caracteres

nsharp

'A'
'ñ'
'🚀'

### Strings

nsharp

"Hello"
"Hola, {name}"
"""
Hola
Mundo
"""

El contenido de un string multilínea conserva exactamente los saltos de línea, incluyendo el salto final antes de `"""`.

### Escapes

N# soporta los siguientes escapes:

text

\n
\t
\r
\\
\"
\'
\u{XXXX}

### Unicode

N# soporta Unicode desde el principio.

### null

nsharp

null

`null` solo es válido para tipos opcionales o tipos referencia.

---

## 9. Tipos

### Tipos primitivos

Los tipos primitivos oficiales son exactamente:

text

$int
$float
$bool
$char
$string
$nothing

Características:

- `$int`: entero estándar. NO existen `$int8`, `$int16`, `$int32`, `$int64`, `$uint`.
    
- `$float`: coma flotante estándar. NO existen `float32` ni `float64` como tipos separados.
    
- `$bool`: `true` / `false`.
    
- `$char`: Unicode scalar/value.
    
- `$string`: string Unicode.
    
- `$nothing`: ausencia de valor de retorno. NO existe un valor `nothing`.
    

### Tipos de usuario

nsharp

let user $User;
let users List<$User>;

Los tipos de usuario NO utilizan `$`.

### Tipos genéricos

nsharp

List<$int>
Map<$string, $int>
Result<$int, $string>
Box<$User>
Rc<$User>
Option<$User>

### Tipos opcionales

nsharp

$int?
$User?
List<$User?>
List<$int>?

La posición de `?` importa:

- `$User?` significa `Option<$User>`.
    
- `List<$User?>` significa `List<Option<$User>>`.
    
- `List<$int>?` significa `Option<List<$int>>`.
    

### Tipos unión

nsharp

$int | $string

### Tipos referencia

nsharp

&$int
&$User

### Tipos puntero

nsharp

*$int
*$User

Solo utilizables en contexto `unsafe`.

---

## 10. Variables y declaraciones

### Declaración con tipo explícito

nsharp

let age $int = 19;
let name $string = "Adrian";
let user $User = User { name: "Adrian", age: 19 };

### Declaración con inferencia

nsharp

age := 19;
name := "Adrian";
user := User { name: "Adrian", age: 19 };

### Declaración sin inicialización

nsharp

let age $int;

Usar una variable antes de haber sido inicializada produce **ERROR DE COMPILACIÓN**.

### Mutabilidad

Todas las variables son mutables. NO existe `mut`. NO existe inmutabilidad forzada.

nsharp

let age $int = 19;
age = 20;

### Cambio de tipo

Una vez establecido el tipo de una variable, este NO PUEDE cambiar.

nsharp

let x $int = 10;
x = "hola";   # ERROR DE COMPILACIÓN

Salvo conversión explícita:

nsharp

let x $int = 10;
let y $float = x as $float;

### Inferencia con null

nsharp

user := null;   # ERROR: cannot infer type of null

### Inferencia con if

nsharp

x := if condition {
    10
} else {
    20
};

→ `$int`

---

## 11. Expresiones

### Expresiones válidas

nsharp

2 + 3 * 4
condition ? a : b
if age >= 18 { "adult" } else { "minor" }
match status { ... }
(x $int) -> $int { return x * 2; }

### Orden de evaluación

El orden de evaluación de operandos DEBE ser de izquierda a derecha, salvo donde se especifique lo contrario.

---

## 12. Operadores y precedencia

### Aritméticos

text

+
-
*
/
%
^

`^` es potencia. NO es XOR.

### Bitwise

text

&
|
^^
~
<<
>>

`^^` es XOR.

### Comparación

text

==
!=
<
>
<=
>=
===

### Lógicos

text

&&
||
!

### Incremento/decremento

text

++
--

### Asignación

text

=
+=
-=
*=
/=
%=

### Ternario

text

condition ? a : b

### Operadores inexistentes

NO existen:

text

and
or
not
??
?.
**

### Tabla de precedencia

|Precedencia|Operador|Asociatividad|Aridad|Significado|
|---|---|---|---|---|
|1|`()` `[]` `.`|izquierda|-|agrupación, indexación, acceso|
|2|`++` `--`|-|unaria|incremento/decremento|
|3|`!` `~` `-` (unario) `&` (ref) `*` (deref)|-|unaria|lógicos/bitwise/ref|
|4|`^`|derecha|binaria|potencia|
|5|`*` `/` `%`|izquierda|binaria|multiplicativos|
|6|`+` `-`|izquierda|binaria|aditivos|
|7|`<<` `>>`|izquierda|binaria|shift|
|8|`<` `<=` `>` `>=`|izquierda|binaria|comparación|
|9|`==` `!=` `===`|izquierda|binaria|igualdad|
|10|`&`|izquierda|binaria|bitwise AND|
|11|`^^`|izquierda|binaria|bitwise XOR|
|12|`\|`|izquierda|binaria|bitwise OR|
|13|`&&`|izquierda|binaria|lógico AND|
|14|`\|`|izquierda|binaria|lógico OR|
|15|`?:`|derecha|ternaria|condicional|
|16|`=` `+=` `-=` `*=` `/=` `%=`|derecha|binaria|asignación|

La precedencia DEBE seguir la de Java salvo donde N# añade operadores propios (`^` como potencia, `^^` como XOR).

---

## 13. Control de flujo

### if / else

nsharp

if age >= 18 {
    println("Adult");
} else if age >= 13 {
    println("Teen");
} else {
    println("Minor");
}

`if` puede ser expresión:

nsharp

let result $string =
    if age >= 18 {
        "adult"
    } else {
        "minor"
    };

Si `if` se utiliza como expresión, `else` es obligatorio.

### while

nsharp

while condition {
    ...
}

### loop

nsharp

loop (i $int = 0; i < 10; i++) {
    println(i);
}

nsharp

loop (i := 0; i < 10; i++) {
}

nsharp

loop (; condition; i++) {
}

nsharp

loop (i := 0; ; i++) {
}

nsharp

loop (;;) {
}

### foreach

nsharp

loop (user $User : users) {
}

o:

nsharp

loop (user : users) {
}

Cualquier tipo que implemente `Iterable<T>` PUEDE utilizarse en foreach: arrays, `List`, `Map`, `Set`, strings.

### break

nsharp

loop (;;) {
    if condition {
        break;
    }
}

### continue

NO existe `continue` en v0.1.

### match

nsharp

match status {
    Status.Active => println("Active");
    Status.Inactive => println("Inactive");
    _ => println("Unknown");
}

`match` es una expresión. DEBE ser exhaustivo.

---

## 14. Funciones

### Sintaxis

nsharp

fn add(a $int, b $int) -> $int {
    return a + b;
}

El tipo de retorno es obligatorio. NO se infiere.

### Función sin retorno

nsharp

fn log() -> nothing {
    println("hello");
    return;
}

`return;` es válido para `-> nothing`. `return nothing;` es inválido.

### Recursión

Permitida.

### Sobrecarga

nsharp

fn print(value $int) -> nothing { }
fn print(value $string) -> nothing { }

El retorno NO participa en overload resolution.

nsharp

fn get() -> $int { }
fn get() -> $string { }   # ERROR DE COMPILACIÓN

### Parámetros opcionales

nsharp

fn greet(name $string?) -> nothing {
}

nsharp

greet();
greet(null);

El argumento omitido se recibe como `null`.

NO existen valores por defecto. NO existen parámetros variádicos en v0.1.

### main

nsharp

fn main() -> $int {
    return 0;
}

o:

nsharp

fn main() -> nothing {
}

`main` tiene exit code 0 por defecto cuando devuelve `nothing`.

---

## 15. Closures y function types

### Function types

text

($int, $int) -> $int

### Almacenamiento

nsharp

let operation = add;

### Función anónima

nsharp

let operation =
    (x $int) -> $int {
        return x * 2;
    };

### Paso como argumento

Las funciones PUEDEN pasarse como argumentos.

nsharp

fn apply(operation ($int) -> $int, value $int) -> $int {
    return operation(value);
}

### Devolución como valor

nsharp

fn createMultiplier() -> ($int) -> $int {
    return (x $int) -> $int {
        return x * 2;
    };
}

---

## 16. Arrays

### Arrays de tamaño fijo

nsharp

let numbers $int[3] = [1, 2, 3];

El tamaño forma parte del tipo. `$int[3]` y `$int[4]` son tipos distintos.

### Inferencia

nsharp

numbers := [1, 2, 3];

→ `$int[3]`

### Array vacío

nsharp

let values $int[0] = [];

### Error de tamaño

nsharp

let values $int[3] = [1, 2];   # ERROR DE COMPILACIÓN

### Arrays multidimensionales

nsharp

matrix := [
    [1, 2],
    [3, 4]
];

→ `$int[2][2]`

### Indexación

nsharp

numbers[0]

### Slices

nsharp

array[1..4]
array[..4]
array[1..]
array[..]

El extremo final es exclusivo.

---

## 17. Collections

N# dispone de:

text

List<T>
Map<K,V>
Set<T>

### List

nsharp

let values List<$int>;

### Map

nsharp

scores := map {
    "Adrian": 100,
    "Carlos": 80
};

### Set

nsharp

numbers := set {
    1,
    2,
    3
};

### Iterabilidad

`List`, `Map`, `Set` implementan `Iterable<T>`.

### Semántica runtime

La representación interna, el crecimiento, la complejidad algorítmica y el layout de memoria de las collections se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 18. Tuplas

### Tuplas heterogéneas

nsharp

user := ("Adrian", 19, true);

Tipo:

text

($string, $int, $bool)

### Acceso

nsharp

user[0]
user[1]
user[2]

NO existe `user.0` en v0.1.

### Destructuring

nsharp

(x $int, y $int) = point;

---

## 19. Structs

### Sintaxis

nsharp

struct User {
    name $string;
    age $int;
}

### Construcción

nsharp

let user = User {
    name: "Adrian",
    age: 19
};

### Acceso

nsharp

user.name

### Visibilidad

Los campos son privados por defecto.

nsharp

pub name $string;

### Mutabilidad

Los campos son mutables.

nsharp

user.age = 20;

---

## 20. Impl blocks

### Métodos

nsharp

impl User {
    fn greet() -> nothing {
        println(self.name);
    }
}

### Múltiples impl

Se permiten múltiples `impl` para el mismo tipo.

### Constructores

NO son una característica especial del lenguaje.

nsharp

impl User {
    fn new(name $string, age $int) -> User {
        return User {
            name: name,
            age: age
        };
    }
}

nsharp

User.new("Adrian", 19);

### Construcción literal

nsharp

User {
    name: "Adrian",
    age: 19
}

---

## 21. Traits

### Declaración

nsharp

trait Drawable {
    fn draw() -> nothing;
}

### Implementación por defecto

nsharp

trait Drawable {
    fn draw() -> nothing {
        println("Drawing");
    }
}

### Implementación

nsharp

impl Drawable for User {
    fn draw() -> nothing {
    }
}

### Múltiples traits

Un tipo PUEDE implementar múltiples traits.

nsharp

impl Drawable for User { }
impl Serializable for User { }

---

## 22. Enums

### Declaración

nsharp

enum Status {
    Active,
    Inactive,
    Pending
}

### Enums con datos

nsharp

enum Result {
    Ok($int),
    Error($string)
}

### Pattern matching

nsharp

match result {
    Result.Ok(value) => println(value);
    Result.Error(message) => println(message);
}

---

## 23. Pattern matching

### Sintaxis

nsharp

match status {
    Status.Active => println("Active");
    Status.Inactive => println("Inactive");
    _ => println("Unknown");
}

### Exhaustividad

El matching DEBE ser exhaustivo. Si no se cubren todas las variantes y no existe `_`:

**ERROR DE COMPILACIÓN**

### Guards

nsharp

match age {
    x if x >= 18 => "adult";
    _ => "minor";
}

### Wildcard

nsharp

_

### Desestructuración

nsharp

match result {
    Result.Ok(value) => println(value);
    Result.Error(message) => println(message);
}

### match como expresión

nsharp

let message $string = match status {
    Status.Active => "Running",
    Status.Inactive => "Stopped",
    _ => "Unknown"
};

---

## 24. Generics

### Struct genérico

nsharp

struct Box<T> {
    value T;
}

### Función genérica

nsharp

fn max<T: Comparable>(a T, b T) -> T {
}

### Constraints

nsharp

<T: Comparable>

### Parámetros genéricos

Los parámetros genéricos NO utilizan `$`:

nsharp

List<$int>
Map<$string, $int>
Result<$int, $string>

### Semántica estática

La monomorfización o estrategia equivalente se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 25. Type aliases

nsharp

type UserId = $int;

El alias es transparente para el type checker.

---

## 26. Union types

nsharp

let value $int | $string;

### Narrowing

El narrowing se realiza mediante `match` o comprobaciones explícitas.

### Compatibilidad

La compatibilidad con generics y con `Option` se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 27. Option

N# posee:

text

Option<T>

con:

text

Some(value)
None

`T?` es azúcar sintáctico para `Option<T>`.

### Ejemplos

nsharp

let name $string? = Some("Adrian");
let user $User? = None;

### Pattern matching

nsharp

match name {
    Some(value) => println(value);
    None => println("No name");
}

---

## 28. Result

N# posee:

text

Result<T, E>

con exactamente:

text

Ok(value)
Error(error)

### Ejemplo

nsharp

fn divide(a $int, b $int) -> Result<$int, $string> {
    if b == 0 {
        return Error("division by zero");
    }
    return Ok(a / b);
}

---

## 29. Error handling

### Propagación con ?

Para `Result`:

nsharp

let value = readUser()?;

Semántica:

text

Ok(v)    → produce v
Error(e) → retorna Error(e) desde la función contenedora

La función contenedora DEBE devolver un `Result` compatible.

Para `Option`:

text

Some(v) → produce v
None   → retorna None

### Excepciones

NO existen excepciones en v0.1.

### panic

nsharp

panic("message");

`panic` termina inmediatamente el proceso.

PUEDE utilizarse en una función `-> nothing`.

---

## 30. Memoria y RAII

N# NO utiliza GC por defecto. Utiliza RAII.

NO existe ownership/borrowing estricto al estilo Rust. NO existe un borrow checker completo.

### Box

nsharp

Box.new(value);

`Box<T>` se libera automáticamente mediante RAII al abandonar su scope.

### Rc

nsharp

Rc.new(value);

`Rc<T>` utiliza reference counting.

### Weak

`Weak<T>` NO existe en v0.1.

### Drop personalizable

nsharp

impl Drop for User {
    fn drop() -> nothing {
    }
}

---

## 31. References

### Sintaxis

nsharp

let ref &$int = &value;

El tipo referencia es:

text

&T

### Referencia

nsharp

&value

### Desreferenciación

nsharp

*ref

### Lifetime

El lifetime y las garantías proporcionadas por N# sin un borrow checker estilo Rust se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 32. Pointers

### Sintaxis

nsharp

*value

Las operaciones peligrosas con punteros solo están permitidas en `unsafe`.

### Distinción

text

reference
pointer
dereference
pointer arithmetic
memory access

---

## 33. unsafe

### Bloques

nsharp

unsafe {
    ...
}

### Funciones

nsharp

unsafe fn readMemory(address $int) -> $int {
}

### Operaciones que requieren unsafe

- dereferenciar punteros crudos;
    
- acceso arbitrario a memoria;
    
- `malloc`;
    
- `free`;
    
- inline assembly;
    
- acceso a registros;
    
- FFI;
    
- operaciones equivalentes de bajo nivel.
    

El compilador DEBE rechazar dichas operaciones fuera de contexto `unsafe`.

---

## 34. const

nsharp

const SIZE $int = 10 * 10;

`const`:

- se evalúa en compile-time;
    
- es inmutable;
    
- no necesita dirección runtime fija.
    

Puede contener cualquier expresión evaluable completamente en compile-time.

nsharp

const NAME $string = getName();   # ERROR si getName() no es compile-time

---

## 35. static

nsharp

static counter $int = 0;

Tiene:

- almacenamiento global;
    
- dirección estable;
    
- lifetime global.
    

Puede ser mutable:

nsharp

counter += 1;

Si se comparte entre threads, requiere sincronización explícita.

La semántica de inicialización y acceso concurrente se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 36. FFI

### Declaración

nsharp

extern "C" {
    fn printf(...);
}

### Definición exportable

nsharp

extern "C" fn myFunction() -> $int {
}

### ABI

El ABI concreto, calling convention, tipos compatibles, ownership, strings, structs, pointers, linking y símbolos externos se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 37. Inline assembly

nsharp

asm!("...");

Solo textual. NO existe una API estructurada de assembly en v0.1.

Solo puede aparecer dentro de `unsafe`.

El tratamiento del compilador y las limitaciones por arquitectura se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 38. Macros

nsharp

macro println($value) {
    ...
}

Las macros operan sobre el AST, NO sobre texto crudo.

Expansión, hygiene, scope, tipos, errores y orden de expansión se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 39. Attributes

nsharp

@derive(Debug)
@derive(Clone)
@derive(Equals)

El compilador procesa attributes conocidos. Attributes desconocidos pueden ser procesados por macros.

NO existe reflexión en v0.1.

---

## 40. Modules

### Import

nsharp

import math;

### Uso

nsharp

math.sqrt(25);

### Import específico

nsharp

import math.sqrt;

### Wildcard

nsharp

import math.*;

### Múltiples

nsharp

import math.sqrt, math.pow;

### Alias

nsharp

import math as m;

La resolución de módulos, nombres y conflictos se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION** en detalle, pero DEBE seguir el modelo de imports descrito.

---

## 41. Packages

### Manifest

toml

name = "example"
version = "1.0.0"
[dependencies]
http = "1.0"

### Git dependency

toml

mylib = { git = "..." }

### Versionado

SemVer: `1.2.3`.

---

## 42. Standard Library

Funciones fundamentales:

nsharp

println(...)
input()
readFile(path)
writeFile(path, data)
args()
env("VAR")

El standard library DEBE incluir conceptualmente:

text

Option
Result
Box
Rc
collections
strings
I/O
filesystem
process
threads
channels
async

La API concreta se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 43. Threads

nsharp

thread.spawn(|| {
    println("Hello");
});

Lifetime, join, sincronización, shared memory, data races y channels se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

NO existen actors en v0.1.

---

## 44. Channels

Los channels forman parte del standard library conceptual. Su API concreta se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 45. Async/Await

### Función async

nsharp

async fn fetch() -> $string {
}

El tipo real de una función async:

text

Future<$string>

### await

nsharp

let result = await fetch();

`await` solo puede utilizarse dentro de `async`.

Future, polling, runtime, executor, cancellation, error propagation e interacción con `?` se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 46. Testing

### Tests integrados

nsharp

test "addition" {
    assert_eq(2 + 2, 4);
}

### Assertions

text

assert
assert_eq

### Benchmarks

text

bench

Los tests del mismo módulo pueden acceder a elementos privados.

Discovery, ejecución, reporting y exit codes se consideran **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 47. Documentation

nsharp

/// Adds two integers.
fn add(a $int, b $int) -> $int {
    return a + b;
}

El comando:

text

ncom doc

genera documentación HTML.

---

## 48. Formatter

text

ncom fmt

DEBE existir un único estilo oficial de formato.

El código NO necesita estar formateado para compilar.

El formatter oficial DEBERÍA ser canónico y determinista.

---

## 49. Compiler diagnostics

Los errores del compilador DEBEN incluir:

- mensaje;
    
- archivo;
    
- línea;
    
- columna;
    
- contexto de código;
    
- diagnóstico coloreado cuando el terminal lo soporte;
    
- warnings;
    
- explicación suficientemente precisa.
    

text

ncom --debug

para diagnósticos/backtraces detallados.

text

ncom --release

para mensajes mínimos de abort/runtime.

Los warnings DEBEN ser configurables.

---

## 50. Project structure

text

project/
├── nsharp.toml
└── src/
    ├── main.ncs
    ├── math.ncs
    └── utils/
        └── strings.ncs

---

## 51. nsharp.toml

toml

name = "example"
version = "1.0.0"
[dependencies]
http = "1.0"

---

## 52. ncom CLI

Comandos oficiales:

text

ncom build
ncom run
ncom check
ncom fmt
ncom test
ncom clean
ncom init
ncom new
ncom install
ncom repl
ncom doc

También:

text

ncom file.ncs

puede compilar/ejecutar directamente.

---

## 53. Script mode

text

ncom script.ncs

En script mode se permite código global.

El compilador genera internamente un `main` implícito.

---

## 54. Project mode

text

ncom build project/

se requiere:

nsharp

fn main() -> $int {
    return 0;
}

o:

nsharp

fn main() -> nothing {
}

NO puede existir simultáneamente código global y `fn main`. Si ambos aparecen en project mode:

**ERROR DE COMPILACIÓN**

---

## 55. Compile-time semantics

### Scopes

DEBEN respetar el modelo de bloques `{}`.

### Symbol tables

DEBEN construirse durante name resolution.

### Name resolution

DEBE resolver identificadores a declaraciones.

### Definite assignment

Una variable DEBE estar inicializada antes de su uso.

### Type compatibility

Los tipos DEBEN ser compatibles según las reglas de esta especificación.

### Inference

DEBE ser local.

### Overload resolution

Solo por parámetros.

### Generic constraints

DEBEN verificarse.

### Trait resolution

DEBE resolverse estáticamente.

### Casts

`as` seguro; `as!` con posible pérdida.

### Option

`T?` = `Option<T>`.

### Result

`Ok` / `Error`.

### ?

DEBE verificarse el tipo de retorno de la función contenedora.

### match exhaustiveness

DEBE verificarse.

### unreachable patterns

DEBEN reportarse como warning o error.

### visibility

`pub` / privado por defecto.

### const evaluation

DEBE ser compile-time.

### unsafe checking

DEBE rechazar operaciones peligrosas fuera de `unsafe`.

### async checking

`await` solo dentro de `async`.

### reference checking

Sin borrow checker completo en v0.1.

---

## 56. Runtime semantics

- Evaluación de expresiones: izquierda a derecha.
    
- Llamadas: por valor.
    
- Retorno: explícito.
    
- Loops: como se describe.
    
- Arrays: tamaño fijo.
    
- Collections: **UNSPECIFIED** en detalle.
    
- RAII: destructores al salir de scope.
    
- Box: liberación automática.
    
- Rc: reference counting.
    
- References: sin garantías formales en v0.1.
    
- static: inicialización **UNSPECIFIED**.
    
- Threads: **UNSPECIFIED**.
    
- async: **UNSPECIFIED**.
    
- Process termination: `exit(code)`, `panic`, retorno de `main`.
    

---

## 57. ABI

El ABI concreto se considera **UNSPECIFIED — RESERVED FOR FUTURE VERSION**.

---

## 58. Formal grammar

ebnf

compilation_unit   ::= { declaration } ;
declaration        ::= function_decl
                     | struct_decl
                     | enum_decl
                     | trait_decl
                     | impl_block
                     | import_decl
                     | const_decl
                     | static_decl
                     | test_decl
                     | attribute_decl
                     | macro_decl
                     | statement ;
function_decl      ::= [ "pub" ] [ "unsafe" ] [ "async" ] "fn" identifier
                       "(" [ param_list ] ")" "->" type block ;
param_list         ::= param { "," param } ;
param              ::= identifier type [ "?" ] ;
struct_decl        ::= [ "pub" ] "struct" identifier [ generic_params ] "{" { field } "}" ;
field              ::= [ "pub" ] identifier type ";" ;
enum_decl          ::= [ "pub" ] "enum" identifier "{" variant { "," variant } "}" ;
variant            ::= identifier [ "(" type { "," type } ")" ] ;
trait_decl         ::= [ "pub" ] "trait" identifier "{" { function_decl } "}" ;
impl_block         ::= "impl" identifier [ "for" identifier ] "{" { function_decl } "}" ;
import_decl        ::= "import" import_path { "," import_path } ";" ;
import_path        ::= identifier { "." identifier } [ ".*" ] [ "as" identifier ] ;
const_decl         ::= "const" identifier type "=" expression ";" ;
static_decl        ::= "static" identifier type "=" expression ";" ;
test_decl          ::= "test" string block ;
attribute_decl     ::= "@" identifier [ "(" attribute_args ")" ] ;
macro_decl         ::= "macro" identifier "(" [ param_list ] ")" block ;
block              ::= "{" { statement } "}" ;
statement          ::= variable_decl
                     | expression_stmt
                     | if_stmt
                     | loop_stmt
                     | while_stmt
                     | break_stmt
                     | return_stmt
                     | match_stmt
                     | unsafe_block ;
variable_decl      ::= "let" identifier [ type ] "=" expression ";"
                     | "let" identifier type ";"
                     | identifier ":=" expression ";" ;
expression_stmt    ::= expression ";" ;
if_stmt            ::= "if" expression block [ "else" ( if_stmt | block ) ] ;
loop_stmt          ::= "loop" "(" [ loop_init ] ";" [ expression ] ";" [ loop_update ] ")" block
                     | "loop" "(" identifier [ type ] ":" expression ")" block ;
loop_init          ::= variable_decl_no_semi ;
loop_update        ::= expression ;
while_stmt         ::= "while" expression block ;
break_stmt         ::= "break" ";" ;
return_stmt        ::= "return" [ expression ] ";" ;
match_stmt         ::= "match" expression "{" match_arm { match_arm } "}" ;
match_arm          ::= pattern [ "if" expression ] "=>" expression ";" ;
unsafe_block       ::= "unsafe" block ;
pattern            ::= "_"
                     | identifier
                     | literal
                     | enum_pattern
                     | tuple_pattern ;
enum_pattern       ::= identifier "." identifier [ "(" pattern { "," pattern } ")" ] ;
tuple_pattern      ::= "(" pattern { "," pattern } ")" ;
expression         ::= assignment_expr ;
assignment_expr    ::= ternary_expr [ assignment_op assignment_expr ] ;
assignment_op      ::= "=" | "+=" | "-=" | "*=" | "/=" | "%=" ;
ternary_expr       ::= logical_or_expr [ "?" expression ":" expression ] ;
logical_or_expr    ::= logical_and_expr { "||" logical_and_expr } ;
logical_and_expr   ::= bitwise_or_expr { "&&" bitwise_or_expr } ;
bitwise_or_expr    ::= bitwise_xor_expr { "|" bitwise_xor_expr } ;
bitwise_xor_expr   ::= bitwise_and_expr { "^^" bitwise_and_expr } ;
bitwise_and_expr   ::= equality_expr { "&" equality_expr } ;
equality_expr      ::= relational_expr { ( "==" | "!=" | "===" ) relational_expr } ;
relational_expr    ::= shift_expr { ( "<" | ">" | "<=" | ">=" ) shift_expr } ;
shift_expr         ::= additive_expr { ( "<<" | ">>" ) additive_expr } ;
additive_expr      ::= multiplicative_expr { ( "+" | "-" ) multiplicative_expr } ;
multiplicative_expr::= power_expr { ( "*" | "/" | "%" ) power_expr } ;
power_expr         ::= unary_expr [ "^" power_expr ] ;
unary_expr         ::= ( "!" | "~" | "-" | "&" | "*" | "++" | "--" ) unary_expr
                     | postfix_expr ;
postfix_expr       ::= primary_expr { postfix_op } ;
postfix_op         ::= "(" [ arg_list ] ")"
                     | "[" expression "]"
                     | "." identifier
                     | "++"
                     | "--" ;
primary_expr       ::= literal
                     | identifier
                     | "(" expression ")"
                     | array_literal
                     | tuple_literal
                     | map_literal
                     | set_literal
                     | if_expr
                     | match_expr
                     | closure_expr
                     | struct_literal ;
if_expr            ::= "if" expression block "else" ( if_expr | block ) ;
match_expr         ::= "match" expression "{" match_arm { match_arm } "}" ;
closure_expr       ::= "(" [ param_list ] ")" "->" type block ;
struct_literal     ::= identifier "{" field_init { "," field_init } "}" ;
field_init         ::= identifier ":" expression ;
array_literal      ::= "[" [ expression { "," expression } ] "]" ;
tuple_literal      ::= "(" expression "," expression { "," expression } ")" ;
map_literal        ::= "map" "{" [ map_entry { "," map_entry } ] "}" ;
map_entry          ::= expression ":" expression ;
set_literal        ::= "set" "{" [ expression { "," expression } ] "}" ;
arg_list           ::= expression { "," expression } ;
type               ::= primitive_type
                     | identifier [ generic_args ]
                     | reference_type
                     | pointer_type
                     | optional_type
                     | union_type
                     | function_type
                     | tuple_type ;
primitive_type     ::= "$int" | "$float" | "$bool" | "$char" | "$string" | "$nothing" ;
generic_args       ::= "<" type { "," type } ">" ;
reference_type     ::= "&" type ;
pointer_type       ::= "*" type ;
optional_type      ::= type "?" ;
union_type         ::= type "|" type ;
function_type      ::= "(" [ type { "," type } ] ")" "->" type ;
tuple_type         ::= "(" type "," type { "," type } ")" ;
literal            ::= integer_literal
                     | float_literal
                     | string_literal
                     | char_literal
                     | "true" | "false" | "null" ;
integer_literal    ::= decimal | hex | binary ;
decimal            ::= digit { digit | "_" } ;
hex                ::= "0x" hex_digit { hex_digit | "_" } ;
binary             ::= "0b" bin_digit { bin_digit | "_" } ;
float_literal      ::= digit { digit } "." digit { digit } ;
string_literal     ::= '"' { char } '"' | '"""' { char } '"""' ;
char_literal       ::= "'" char "'" ;

### GRAMMAR AMBIGUITIES

1. `map { ... }` vs `set { ... }` vs struct literal: resuelto por la keyword `map` / `set`.
    
2. `loop (identifier : expression)` vs `loop (init; cond; update)`: resuelto por la presencia de `;`.
    
3. `if` como expresión vs statement: resuelto por contexto (asignación o retorno).
    
4. `match` como expresión vs statement: resuelto por contexto.
    
5. `(x $int) -> $int { ... }` vs agrupación: resuelto por la presencia de `->` tras `)`.
    

---

## 59. Reserved/future features

Las siguientes características NO forman parte de N# v0.1:

text

continue
switch
class
interface
extends
mut
throw
catch
try
this
default parameters
variadic parameters
Weak<T>
reflection
actors
custom operators
custom primitive integer widths
unsigned integer types
scientific notation
octal literals

---

## 60. Appendix: ejemplos completos

### Hello World

nsharp

fn main() -> nothing {
    println("Hello, World!");
}

### CLI

nsharp

fn main() -> $int {
    let arguments List<$string> = args();
    if arguments.length() == 0 {
        println("No arguments");
        return 1;
    }
    loop (arg : arguments) {
        println(arg);
    }
    return 0;
}

### Struct + trait

nsharp

trait Drawable {
    fn draw() -> nothing;
}
struct Circle {
    radius $float;
}
impl Drawable for Circle {
    fn draw() -> nothing {
        println("Drawing circle");
    }
}
fn main() -> nothing {
    let c = Circle { radius: 3.14 };
    c.draw();
}

### Result

nsharp

fn divide(a $int, b $int) -> Result<$int, $string> {
    if b == 0 {
        return Error("division by zero");
    }
    return Ok(a / b);
}
fn main() -> nothing {
    let result = divide(10, 2)?;
    println(result);
}

### Option

nsharp

fn findUser(id $int) -> Option<$string> {
    if id == 1 {
        return Some("Adrian");
    }
    return None;
}
fn main() -> nothing {
    match findUser(1) {
        Some(name) => println(name);
        None => println("Not found");
    }
}

### Match

nsharp

enum Status {
    Active,
    Inactive,
    Pending
}
fn main() -> nothing {
    let status = Status.Active;
    let message $string = match status {
        Status.Active => "Running";
        Status.Inactive => "Stopped";
        Status.Pending => "Waiting";
    };
    println(message);
}

### Generics

nsharp

fn max<T: Comparable>(a T, b T) -> T {
    if a > b {
        return a;
    }
    return b;
}
fn main() -> nothing {
    println(max(10, 20));
}

### Collections

nsharp

fn main() -> nothing {
    let numbers List<$int>;
    numbers.add(1);
    numbers.add(2);
    numbers.add(3);
    let scores = map {
        "Adrian": 100,
        "Carlos": 80
    };
    let unique = set {
        1,
        2,
        3
    };
    println(numbers[0]);
    println(scores["Adrian"]);
    println(unique);
}

### Async

nsharp

async fn fetch() -> $string {
    return "data";
}
async fn mainAsync() -> nothing {
    let result = await fetch();
    println(result);
}

### Unsafe

nsharp

fn main() -> nothing {
    unsafe {
        let ptr *$int = malloc(4) as *$int;
        *ptr = 42;
        println(*ptr);
        free(ptr);
    }
}

### FFI

nsharp

extern "C" {
    fn printf(format $string, ...) -> $int;
}
fn main() -> nothing {
    unsafe {
        printf("Hello from C\n");
    }
}

---

## VALID / INVALID PROGRAMS

### VÁLIDO

nsharp

let age $int = 19;
age = 20;

### INVÁLIDO

nsharp

user := null;

**ERROR DE COMPILACIÓN:** cannot infer type of null.

nsharp

return nothing;

**ERROR DE COMPILACIÓN:** `nothing` no es un valor.

nsharp

10 === 10;

**ERROR DE COMPILACIÓN:** `===` solo sobre tipos referencia.

nsharp

fn get() -> $int { }
fn get() -> $string { }

**ERROR DE COMPILACIÓN:** overload ambiguo; el retorno no participa.

nsharp

let values $int[3] = [1, 2];

**ERROR DE COMPILACIÓN:** tamaño de array incompatible.

nsharp

let x $int;
println(x);

**ERROR DE COMPILACIÓN:** variable no inicializada.

nsharp

unsafeOperation();

**ERROR DE COMPILACIÓN:** operación peligrosa fuera de `unsafe`.

nsharp

match status {
    Status.Active => println("active");
}

**ERROR DE COMPILACIÓN:** match no exhaustivo.

nsharp

await fetch();

**ERROR DE COMPILACIÓN:** `await` fuera de `async`.

nsharp

let x $int = null;

**ERROR DE COMPILACIÓN:** `null` en tipo no opcional.

---

## Consistency check

- `$User?`, `List<$User?>`, `List<$int>?`, `Option<$User>`, `Result<$int,$string>` son coherentes.
    
- `x := null;` es inválido.
    
- `return;` es válido para `nothing`; `return nothing;` es inválido.
    
- `match` es exhaustivo.
    
- `T?` = `Option<T>`.
    
- `Ok` / `Error` son las únicas variantes estándar de `Result`.
    
- `?` funciona para `Result` y `Option`.
    
- `&T` es la sintaxis de referencia.
    
- `*ptr` requiere `unsafe` cuando corresponda.
    
- `===` no puede utilizarse con value types.
    
- `new` NO es keyword.
    
- `_` es wildcard reservado.
    
- `loop` admite todas las formas especificadas.
    
- `let` es keyword oficial.