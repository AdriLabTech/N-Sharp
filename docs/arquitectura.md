# Arquitectura de NCompiler

```
src/
├── diag.rs        Span, SourceMap y diagnósticos con contexto (color si la terminal lo soporta)
├── lexer.rs       Tokens, comentarios anidables `!# #!`, strings con interpolación, `///`
├── ast.rs         AST (+ recorridos); el checker lo reescribe (coerciones, resoluciones)
├── parser.rs      Descenso recursivo + reglas de desambiguación del §58
├── types.rs       `Ty`: tipos semánticos
├── program.rs     Carga de módulos: `core` (prelude), módulo principal, imports, std, dependencias
├── check/         Análisis semántico
│   ├── decls.rs     recolección de tipos/traits/impls/firmas, imports, resolución de tipos
│   ├── body.rs      items, cuerpos de función, ámbitos, bloques, sentencias, asignación definida
│   ├── expr.rs      tipado de expresiones y operadores
│   ├── call.rs      llamadas, sobrecargas, métodos, rutas `modulo.item` / `Tipo.item`
│   ├── unify.rs     variables de inferencia, unificación con deshacer, coerciones, cotas
│   └── pat.rs       patrones, exhaustividad y patrones inalcanzables (algoritmo de matrices)
├── interp/        Backend intérprete
│   ├── value.rs     `Value` (Rc + copy-on-write), `Scope`, `Place` (variable + camino)
│   ├── mod.rs       carga de tablas, llamadas, RAII, bloques
│   ├── eval.rs      expresiones, patrones, operadores, conversiones
│   ├── intrinsics.rs  funciones del runtime (colecciones, strings, math, fs, hilos, ...)
│   ├── display.rs   formato de valores
│   └── runner.rs    `test`, `bench` y ejecución de sentencias globales (REPL)
├── fmt.rs         Formateador canónico (AST + comentarios reinyectados)
├── doc.rs         Generador de documentación HTML
├── project.rs     nsharp.toml, dependencias, andamiaje y ejecutables empaquetados
├── repl.rs        REPL
├── cli.rs         Comandos de `ncom`
└── main.rs
std/               Biblioteca estándar escrita en N# (`core.ncs` = prelude; el resto son módulos)
```

## Flujo

1. **Carga** (`program::Loader`): parsea `std/core.ncs` y el módulo principal; resuelve `import` buscando primero módulos de
   usuario (`src/…`, dependencias) y después la biblioteca estándar. Los módulos estándar usados sin `import` se cargan también.
2. **Análisis** (`check::check_program`), en pasadas:
   1. nombres de tipos, funciones y constantes de todos los módulos;
   2. imports (alias, elementos concretos, `.*`);
   3. campos de structs, variantes de enums, genéricos y cotas;
   4. firmas de traits, `impl`, funciones y constantes (y completitud de los `impl Trait for T`);
   5. cuerpos: inferencia local con variables de unificación, coerciones (`T → T?`, `[..] → List`), resolución de
      sobrecargas y métodos, asignación definida, `unsafe`/`async`, visibilidad, exhaustividad de `match`, retorno en todos los caminos.
   El checker **reescribe el AST**: nombres resueltos (`Global`, `TypeStatic`, `VariantRef`), `SomeWrap`, argumentos
   opcionales omitidos (`null`) y colecciones sin inicializador (`let xs List<$int>;` → `List.new()`).
3. **Ejecución** (`interp`): el AST verificado vive en memoria estática; los valores usan `Rc` con copy-on-write
   (semántica de valor), las referencias son `Place`s (celda + camino de campos/índices) y los destructores RAII se
   disparan al salir de los ámbitos cuando el valor no está compartido.

## La biblioteca estándar como código N#

`std/core.ncs` declara los tipos del runtime (`List`, `Map`, `Set`, `Box`, `Rc`, `Option`, `Result`, …) y sus métodos.
Las funciones **sin cuerpo** son intrínsecas: el intérprete las enlaza por nombre (`List.add`, `$string.split`,
`math.sqrt`, …) al cargar el programa y falla con una lista si falta alguna. Las demás (`Option.map`, `List.filter`,
`Map.get_or`, …) están escritas en N#, por lo que sirven también como prueba del propio compilador.

## Pruebas

- Unitarias: lexer, parser, formateador, documentación, proyectos.
- `tests/examples.rs`: cada `examples/*.ncs` se ejecuta y se compara con su `.out`/`.code`; cada `tests/invalid/*.ncs`
  debe ser rechazado con el error de su primera línea; el formateador debe ser idempotente y no cambiar el comportamiento.
- `tests/cli.rs`: proyectos, módulos, visibilidad, dependencias, `build`, `fmt`, `test`, `doc`, REPL, E/S.
