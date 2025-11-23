| world     | i/e    | name            | type               | impl                                                         |
| --------- | ------ | --------------- | ------------------ | ------------------------------------------------------------ |
| core      | import |                 |                    |                                                              |
| core      | export | sock            | interface          | 1. package:interface::{Guest as SockGuest}<br/>2. package:interface::{GuestSock}<br/>3. impl SockGuest for Core {} |
| core      | export | sock            | interface/resource | 1. struct SockSock;<br/>2. impl GuesSock for SockSock {} <br/> |
| core      | export | log             | interface/func     |                                                              |
| core-libs | export | user-state      | interface/record   | 无需实现                                                     |
| core-lib  | export | core-call-model | interface          | 1. struct  CoreLib;<br/>2. package:interface::Guest<br/>3. impl Guest for CoreLib<br/> |

