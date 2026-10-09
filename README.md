# N# (N Sharp)

**N#** es un lenguaje de programación generalista, estáticamente tipado, con inferencia local, RAII,
traits, pattern matching y `unsafe` explícito. Este repositorio contiene **NCompiler** (la implementación
de referencia de la [especificación v0.1](docs/especificacion.md)) y su CLI **`ncom`**.

```nsharp
trait Drawable {
    fn draw() -> nothing;
}

struct Circle {
    radius $float;
}

impl Drawable for Circle {
    fn draw() -> nothing {
        println("Dibujando un círculo de radio {self.radius}");
    }
}

fn divide(a $int, b $int) -> Result<$int, $string> {
    if b == 0 {
        return Error("división por cero");
    }
    return Ok(a / b);
}

fn main() -> $int {
    Circle { radius: 3.14 }.draw();
    match divide(10, 2) {
        Ok(v) => println("10 / 2 = {v}");
        Error(e) => println("error: {e}");
    }
    return 0;
}
```

## Estado

NCompiler implementa **toda la superficie del lenguaje** definida en la especificación v0.1 (lexer, parser,
análisis semántico completo, biblioteca estándar, CLI, formateador, documentación, tests integrados,
REPL y gestión de paquetes). La especificación deja explícitamente **RESERVADO** el backend de código
nativo, el IR, el runtime, el ABI y la convención de llamada; por eso la ejecución se hace con un
**intérprete de AST** y `ncom build` produce un **ejecutable autónomo** (el intérprete con el programa
incrustado) en lugar de código máquina. Los detalles y todas las decisiones tomadas donde la
especificación calla están en [docs/decisiones.md](docs/decisiones.md).

## Instalación

Requiere Rust (edición 2024, probado con 1.99). No tiene dependencias externas.

```sh
cargo build --release
./target/release/ncom --version        # ncom 0.1.0 (N# v0.1)
# opcional: instalar en ~/.cargo/bin
cargo install --path .
```

## Uso rápido

```sh
ncom hola.ncs                 # script: compila y ejecuta (acepta código global)
ncom new miproyecto           # crea nsharp.toml + src/main.ncs
cd miproyecto
ncom run                      # compila y ejecuta el proyecto
ncom build                    # genera target/ncom/miproyecto (ejecutable autónomo)
ncom check                    # solo análisis, sin ejecutar
ncom test                     # ejecuta los bloques `test` (ncom test --bench: los `bench`)
ncom fmt                      # formatea con el estilo oficial (--check para solo comprobar)
ncom doc                      # documentación HTML en target/doc
ncom repl                     # intérprete interactivo
```

Opciones útiles: `--debug` (backtrace completo en los pánicos), `--release` (mensajes mínimos),
`-A <categoría>` / `-D <categoría>` (silenciar / convertir en error un warning; categorías:
`unused-variable`, `unreachable-code`, `unreachable-pattern`, o `warnings` para todas).

Todo lo que sigue a un archivo `.ncs` son argumentos del programa (`args()`).

## Estructura de un proyecto

```
miproyecto/
├── nsharp.toml
└── src/
    ├── main.ncs
    ├── geometria.ncs        # import geometria;
    └── utils/
        └── texto.ncs        # import utils.texto;
```

```toml
name = "miproyecto"
version = "1.0.0"

[dependencies]
utiles = { git = "https://github.com/usuario/utiles" }   # ncom install
local  = { path = "../local" }

[warnings]
unused-variable = "allow"      # "allow" | "warn" | "deny"
```

## Un recorrido por el lenguaje

| Tema | Ejemplo |
|---|---|
| Tipos primitivos | `$int` `$float` `$bool` `$char` `$string` `nothing` |
| Variables | `let edad $int = 19;` · `nombre := "Adrián";` · `let x $int;` |
| Opcionales | `$int?` = `Option<$int>` · `Some(1)` · `None` · `null` · `x?` propaga |
| Colecciones | `List<$int>` · `map { "a": 1 }` · `set { 1, 2 }` · arrays `$int[3]` |
| Control de flujo | `if`/`else` (expresión) · `while` · `loop (i := 0; i < 10; i++)` · `loop (x : xs)` · `match` |
| Funciones | `fn suma(a $int, b $int) -> $int { ... }` · sobrecarga por parámetros · clausuras |
| Tipos de usuario | `struct` · `enum` con datos · `impl` · `trait` (con métodos por defecto) · genéricos con cotas |
| Memoria | RAII · `Box.new(v)` · `Rc.new(v)` · `impl Drop for T` · `&T` referencias |
| Bajo nivel | `unsafe { ... }` · punteros `*$int` · `malloc`/`free` · `extern "C"` |
| Concurrencia | `thread.spawn(\|\| { ... })` · `channel.new()` · `async fn` / `await` |
| Módulos | `import math;` · `import math.sqrt;` · `import math.*;` · `import math as m;` |
| Tests | `test "suma" { assert_eq(1 + 1, 2); }` |

Hay ejemplos ejecutables de cada punto en [`examples/`](examples) (cada `.ncs` tiene su salida esperada
en el `.out` vecino) y programas que **deben** rechazarse en [`tests/invalid/`](tests/invalid).

## Biblioteca estándar

Siempre disponible: `println` `print` `eprintln` `input` `readFile` `writeFile` `args` `env` `to_string`
`assert_eq` `assert_ne` `block_on`, los tipos `Option` `Result` `List` `Map` `Set` `Box` `Rc` y los
traits `Comparable` `Numeric` `Iterable<T>` `Drop` `Display`.
Módulos (con o sin `import`): `math` `time` `thread` `channel` `fs` `io` `process`.
Las firmas están en [`std/*.ncs`](std) escritas en el propio N#; las funciones sin cuerpo son
intrínsecas del runtime.

## Arquitectura

```
fuente .ncs → Lexer → Parser → AST → resolución de nombres + tipos (check/) → intérprete (interp/)
                                                │
                                      ncom fmt / ncom doc / REPL reutilizan el AST
```

Detalles en [docs/arquitectura.md](docs/arquitectura.md).

## Desarrollo

```sh
cargo test        # unitarias + 26 ejemplos + 40 programas inválidos + CLI + formateador
```

## Documentación

- [Especificación v0.1](docs/especificacion.md) (copia del texto normativo)
- [Decisiones de implementación y límites](docs/decisiones.md)
- [Arquitectura](docs/arquitectura.md)
