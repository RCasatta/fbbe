use super::html_page;
use crate::{error::Error, pages::NBSP};
use maud::{html, Markup};

pub fn page() -> Result<Markup, Error> {
    let content = html! {
        section {
            h1 { "Contact" }

            form action="https://pay2.email" method="POST" {
                label {
                    p { "Your email:"}
                    input type="email" name="reply_to" { }
                }
                p { (NBSP) }
                label {
                    p { "Your message:"}
                    textarea name="message" rows="4" cols="50" { }
                }
                input type="hidden" name="to_enc" value="e1v9nk2tt9de3hy7tsw35k7m3wdaexwtmkxy9z603qtqer2df38ysyzamfdvmn2n2zvav8xjr8x3hrxwrtty69s7p3wf3454rcwaxnwjjjf95xg4mrgfz8q46ppfy5zsmkxfx9s6mdx3xhx7350fryu5nwx4v9znj0fpfnx62pwff85wrrw44y652r2fn9s3g295lzqcfgt45j6emjv4shxefq9pcyugreyv58k46ppfyngerfw4qkg4jr09f8xdjw2dgngnm0guc566thx4rhvu6vfa8k7dztdeprgw2ev9yxj4tyw45yvnnztpfny72ywsmrze6r24knsa6kpfp4gnejxemxwjzz2u4nsn3c2atx54j32q4hxmteg3z56n6vw34n2vekfe68wn3kwyuxz7n3dpns5tfd95s9jdjdwue4s42s9atycj3cdqm9snjvxasnyumtfcuxu3jdfu45x3zwvfyy2une2py5cnr0p2vuywu4w95y5xuve803rmeq49hevxglyraqqh9a0q9ae6vwfkshep574e7fm4uujwnaz87jpzfmvdghlac9zj5sxanhrkckny02n" { }
                input type="hidden" name="subject_enc" value="e1v9nk2tt9de3hy7tsw35k7m3wdaexwtmkxy9z603qtqer2df38ys8vurcfacnssnxv9kkw3rewdmx5d2tt9zrswr82athyw25d35x6660fscn2dzh2388z4j3pg4kvmr6v3u5sv3429gnq3t6xum8smmxw4t57jttd3ckgvr5v9r5uuj4w4u5jsm0gccxyng295lzqkzg9qhkctt8wfjkzum9yqexvgedwarzqdf9tfy9x4pqtse9qgj4d4zq54j2ffgq5tfd95sx5ur3f494ja68dprrzajyggu5c3f4xsmnq53tvguyy7j5wef4g3trgsunjwzkv458vdtrpt7vk50lxwmr2prerxtjydexs43kculrqazt3va0xhjp6w07jmqwlnzt9jx54mskf0rsc22n2t" { }
                p { (NBSP) }
                button type="submit" { "Pay 20 satoshi ⚡ to send" }
                p { (NBSP) }
            }

        }
    };

    Ok(html_page("Contact", content))
}
