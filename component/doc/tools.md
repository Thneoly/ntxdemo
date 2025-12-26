| world     | i/e    | name            | type               | impl                                                         |
| --------- | ------ | --------------- | ------------------ | ------------------------------------------------------------ |
| core      | import |                 |                    |                                                              |
| core      | export | sock            | interface          | 1. package:interface::{Guest as SockGuest}<br/>2. package:interface::{GuestSock}<br/>3. impl SockGuest for Core {} |
| core      | export | sock            | interface/resource | 1. struct SockSock;<br/>2. impl GuesSock for SockSock {} <br/> |
| core      | export | log             | interface/func     |                                                              |
| core-libs | export | user-state      | interface/record   | 无需实现                                                     |
| core-lib  | export | core-call-model | interface          | 1. struct  CoreLib;<br/>2. package:interface::Guest<br/>3. impl Guest for CoreLib<br/> |



规则

> wit中 `world` export `interface` 分为几种情况
>
> 1. `interface` 中有 `func`, impl时需要 为 `world` 实现一个`trait`, `trait`名称为`Guest` 路径为: `package::/interface::Guest`, `Guest` 中需要实现 `func`
> 2. `interface`中有 `resource`, impl时 需要为`world` 指定一个`type resouece= Struct`, `Struct` 需要为`world`实现一个`trait`, `trait`名称为 `GuestResource`, 路径为 `package::interface::GuestResource`。`trait`需要实现`resource` 中的`func`。 如果函数签名中使用到了`resource` 需要导入`resource`， 路径为 `package::interface::Resource`.
> 3. `interface`中有 `resource`时，当有`func` 返回 `resource`时， 在 `world` 实现时， 需要定义 `type Res = Resource`, `Resource` 为 `resource`对应的`Struct`，需要导入， 然后返回时 返回`Resource::new(ResourceStruct)`;
> 4. 函数定义中出现 borrow<type>时，需要从`package::interface`中导入`TypeBorrow`
> 5. `interface`中 没有`func`和`resource` 此时应该不用实现??


wac规则：
// WAC composition that wires eventbus, scheduler, and actions-executor together.
// 使用方式示例（在仓库根目录执行）：
//   wac compose component/wac/scheduler-composition.wac \
//     --deps-dir component/wac/deps \
//     -o component/wac/scheduler-composed.wasm

1. 组件放到deps/name1/component-name.wasm // component-name 不能使用下划线，只能使用中划线
2. let cmp1 = new ${name1}:component-name{...}; // component-name 不带版本号
3. let cmp2 = new component:action-executor {
    "ntx:scenario-eventbus/event-bus@0.1.0":  // wit 文件中的use
        eventbus["ntx:scenario-eventbus/event-bus@0.1.0"], // eventbus 中的package名称
    ...
};
4. export cmp2 as cmp2-name; 
4-1. 给 host 用的导出：export scheduler as escheduler; 现在是正确的，组合后的 wasm 已经能被宿主按 escheduler 这个实例加载使用。
4-2. 给 wasm-tools component wit 看 WIT：当前这条 export 形式，本质是「导出一个 instance」，wasm-tools component wit 目前只会尝试把「函数 / 类型导出」还原为 WIT，对 instance 导出支持很差，所以才会报你看到的错误。
5. export cmp2["some"]; # some 是 cmp2 对应的world中的一个export;
6. export 可以有多个。



wasmtime bindgen::
1. 对于world中的import 需要实现Host？ export 实现Guest？



1. Wasm 模块中的 import 和 export

import：表示当前 WebAssembly 模块依赖于外部（比如主机或者其他模块）提供的功能。

例如，如果 WebAssembly 模块需要调用主机提供的文件系统功能或获取时间戳等操作，模块就需要在 WIT 文件中声明 import，告诉编译器这些功能是由主机提供的，模块将依赖它们。

export：表示当前 WebAssembly 模块暴露给外部（比如主机或其他模块）使用的功能。

例如，如果 WebAssembly 模块内部实现了一个加法函数，并且希望主机能够调用这个函数，那么模块就会在 WIT 文件中使用 export 来声明这个函数，允许外部访问它。

2. Host 端的 import 和 export

对于主机端的实现，思路和方向与 WebAssembly 模块端相反：

Host 需要实现 import：这是因为从主机的角度来看，它要向 WebAssembly 模块提供一些功能或资源，供模块调用。

例如，如果主机要提供文件读取、网络连接等能力，主机应该在 WIT 文件中声明 import，表示主机提供了这些能力，WebAssembly 模块需要使用它们。

Host 需要使用 export：如果主机端需要调用 WebAssembly 模块的功能或暴露自己实现的某些功能（比如将某些主机的操作暴露给其他系统使用），则主机将使用 export 来声明。

例如，如果主机需要调用 WebAssembly 模块的一个计算函数（比如加法函数），主机会在 WIT 文件中使用 import 来引入 WebAssembly 模块的该函数。

总结您的理解：

WebAssembly 模块：

需要调用外部接口（比如主机或其他模块提供的功能）时，使用 import。

暴露接口给外部（比如主机或其他模块调用自己实现的功能）时，使用 export。

Host 端：

提供接口供 WebAssembly 模块调用时，使用 import。

调用 WebAssembly 模块的接口时，使用 import。

暴露接口供其他系统使用时，使用 export。

实际开发中的应用：

在开发过程中，如果您是在 WebAssembly 模块 中编写代码，则您会在 WIT 文件中使用 import 来声明主机提供的功能，使用 export 来暴露模块的功能。

在 主机端 实现时，您会根据主机的需求来声明 import（给 WebAssembly 模块提供功能）和 export（主机需要使用模块的功能）。

这样一来，import 和 export 确保了 WebAssembly 模块和主机之间的清晰边界和相互操作。