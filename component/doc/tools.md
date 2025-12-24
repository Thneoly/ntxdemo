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