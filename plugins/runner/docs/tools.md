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