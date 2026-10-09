# Decisiones de implementación y límites

La especificación v0.1 define la *superficie* del lenguaje y deja muchas cosas como
**UNSPECIFIED — RESERVED FOR FUTURE VERSION**. Este documento recoge, en un solo sitio, cómo las resuelve
NCompiler, qué extensiones menores añade y qué límites tiene la implementación actual.

## 1. Backend

| Spec | Implementación |
|---|---|
| Generación de código nativo, IR, runtime, ABI, calling convention (§3, §57) | **Reservado.** Se ejecuta con un intérprete de AST sobre el programa ya verificado. |
| `ncom build` (§52) | Produce un **ejecutable autónomo**: una copia de `ncom` con las fuentes del programa incrustadas al final. No es código máquina, pero se distribuye y se ejecuta como un binario normal (acepta `args()`, devuelve el código de salida de `main`). |
| Rendimiento | Orientativo: `fib(32)` ≈ 1,2 s en una máquina de desarrollo. Un backend nativo/bytecode queda para versiones futuras. |

## 2. Decisiones donde la especificación calla

- **`$int`** es un entero con signo de **64 bits**; el desbordamiento aritmético (`+ - * ^ -x`) provoca un pánico
  (`desbordamiento aritmético`). `/` y `%` entre cero provocan pánico. `$float` es IEEE-754 doble.
- **Strings multilínea** (`"""`): se descarta el salto de línea inmediatamente posterior a la apertura; se conserva el
  último salto antes del cierre (así `"""\nHola\nMundo\n"""` vale `"Hola\nMundo\n"`).
- **Llaves en strings:** `{expr}` interpola; para una llave literal se escribe `\{` y `\}` (extensión de la tabla de
  escapes del §8, necesaria porque la interpolación del §8 no define otra salida).
- **`?` postfijo vs. ternario:** `a?` (pegado al operando anterior) propaga errores; `c ? a : b` (con espacio) es el ternario.
- **Métodos:** `self` es implícito en los `impl` (el §20 no lo declara como parámetro). Un método que **usa** `self` es de
  instancia y solo se llama sobre un valor; uno que no lo usa (p. ej. `new`) también puede llamarse como `Tipo.nombre(..)`.
  Los métodos intrínsecos sin cuerpo son de instancia salvo `new`.
- **Semántica de valor:** estructs, tuplas, arrays, `List`, `Map` y `Set` se copian al asignarse o pasarse (implementado con
  copy-on-write, así que copiar es O(1) hasta que se modifica). Para compartir se usa `&T`, `Box<T>` o `Rc<T>`.
  `&x` es una referencia a la variable `x` (no a su valor); `*r` la desreferencia y puede asignarse (`*r = 5;`).
  `Box<T>` y `Rc<T>` son celdas en el montón: copiar el valor comparte la celda (`===` compara identidad).
- **RAII / `Drop`:** los destructores se ejecutan al salir del ámbito, las variables en orden inverso de declaración (campos y elementos de colecciones, de principio a fin, como en Rust), **solo si el valor
  no está compartido** (una variable «movida» a otra no se destruye dos veces). No se destruye al sobrescribir una
  variable, ni los temporales, ni cuando el programa termina por `panic`/`exit`.
- **Inicialización de colecciones:** `let xs List<$int>;` (sin valor) crea una lista vacía (así lo muestra el §60); lo mismo
  para `Map` y `Set`. Para cualquier otro tipo, usar una variable sin inicializar sigue siendo un error de compilación.
- **Opcionales:** `T` se convierte implícitamente en `T?` (`let n $int? = 5;`, `greet("Ana")` con parámetro `$string?`);
  `null` equivale a `None` y es válido en `T?` y en referencias. Los parámetros opcionales finales pueden omitirse.
- **`main` con `?`:** el §29 exige que la función que usa `?` devuelva un `Result`, pero el ejemplo `Result` del §60
  usa `?` dentro de `fn main() -> nothing`. NCompiler admite `?` sobre `Result` en `main`: si hay un error, imprime
  `Error: <mensaje>` y termina con código 1.
- **Sobrecarga:** se resuelve estáticamente por parámetros; si el retorno es lo único que difiere, es un error (§14).
  En ejecución se vuelve a elegir por el tipo dinámico de los argumentos.
- **Uniones (`$int | $string`):** se estrechan con `match` y *patrones con tipo*: `n $int => ...` (extensión del §26, que solo
  dice «mediante `match`»). El `match` debe cubrir todas las alternativas o tener `_`.
- **Traits:** solo genéricos con cotas (`<T: Trait>`); no hay objetos-trait (`dyn`). Los traits de la biblioteca:
  `Comparable` (`compare(other Self) -> $int`, habilita `< > <= >=` y `sort`), `Numeric`, `Iterable<T>`
  (`items() -> List<T>`, habilita `loop`), `Drop`, `Display` (`to_string() -> $string`, usado por `println` e interpolación).
- **Slices:** `array[a..b]` produce una `List<T>` (el tamaño de un slice no es conocido en compilación); en strings produce `$string`.
- **Literal de array → `List<T>`:** un literal `[1, 2, 3]` se acepta donde se espera una `List<$int>`.
- **Módulos estándar sin `import`:** `thread.spawn(..)` (§43) funciona sin importar; `import thread;` también es válido.
- **Visibilidad:** campos y funciones son privados por defecto *respecto de otros módulos*; dentro del mismo módulo (y en sus `test`) todo es accesible.
- **Código global:** solo en script mode (`ncom archivo.ncs`); sus variables son locales del `main` implícito y no las ven las funciones.
- **Warnings:** `unused-variable`, `unreachable-code`, `unreachable-pattern`; configurables con `-A/-D` y en `[warnings]` de `nsharp.toml`.
- **`for`** no es palabra reservada (el §6 no la lista) aunque `impl Trait for Tipo` la usa como palabra contextual.
- **Closures:** además de `(x $int) -> $int { ... }` se acepta la forma corta `|x| expr` / `|| { ... }` que usa el §43.

## 3. Límites conocidos (zonas que el §59 deja fuera o la spec marca como UNSPECIFIED)

- **Hilos (§43/§44):** cooperativos. `thread.spawn` encola la tarea; se ejecuta completa al hacer `join()`, al bloquear un
  `recv()` sin mensajes o al terminar `main`. Un hilo no puede quedarse esperando a otro (no hay corrutinas); sin
  paralelismo real ni *data races*.
- **Async (§45):** los `Future` son perezosos; se ejecutan al hacer `await` o `block_on`. No hay ejecutor concurrente.
- **FFI (§36):** el intérprete no carga bibliotecas C. Se emula un subconjunto de libc: `printf` (con `%d %i %u %x %X %o %c %s %f %e %g %p %%`),
  `puts`, `putchar`, `abs`, `labs`, `strlen`, `getpid`, `exit`, `sqrt`, `pow`. Declarar otras funciones `extern` es válido,
  pero llamarlas produce un pánico explicativo.
- **Memoria `unsafe` (§32/§33):** el montón es simulado y con comprobaciones (puntero nulo, fuera de límites, uso tras
  `free`, doble `free`). `malloc(n)` reserva `n` posiciones (una por «byte» pedido) y la aritmética de punteros
  `p + k` avanza de elemento en elemento.
- **Ensamblador en línea (§37):** se analiza (solo dentro de `unsafe`) pero no se ejecuta.
- **Macros (§38):** se analizan y se rechazan con un error claro (semántica UNSPECIFIED).
- **Reflexión, actores, `Weak<T>`, `continue`, `switch`, ...:** fuera de v0.1 (§59). Usarlos produce un error que lo explica.
- **Dependencias por versión** (`http = "1.0"`): no existe aún un registro; `ncom` lo explica. Funcionan `{ git = "..." }` y `{ path = "..." }`.
- **Enteros:** el literal `-9223372036854775808` no se puede escribir (el 9223372036854775808 no cabe en `$int` antes de negarlo).
- **`List<$float>.sum()`** de una lista vacía devuelve `0` (entero) porque el runtime no conoce el tipo de los elementos.
- **REPL:** `static` y los efectos secundarios de las declaraciones no se conservan entre líneas (cada línea se verifica
  junto con el historial, pero solo se ejecuta lo nuevo; las variables sí persisten).
