# lambstock [![Build Status](https://travis-ci.org/meetup/lambstock.svg?branch=master)](https://travis-ci.org/meetup/lambstock) [![Coverage Status](https://coveralls.io/repos/github/meetup/lambstock/badge.svg)](https://coveralls.io/github/meetup/lambstock)

> organizational aws lambda discovery from the command line 🐑 🐑 🐑 🐓🐏 🐕🐑

## 🤔 about

In order to make managing applications easier AWS lambda greatly
improves engineer effiency in writing small applications that are easier to understand and operate.
It also creates a [jevon's paradox](https://en.wikipedia.org/wiki/Jevons_paradox) where by
applications are now so easy to create that there are now many many more applications the manage
introducing different kinds of application management problems, discovery problems.

Lambstock is a discovery tool for AWS lambda to that enables you to explore
your lambda stock of applications from the command line

# 📦 install

## Github releases

Prebuilt binaries for osx and linux are available for download directly from [Github Releases](https://github.com/meetup/lev/releases)

```bash
$ curl -L \
 "https://github.com/meetup/lambstock/releases/download/v0.0.0/lambstock-v0.0.0-$(uname -s)-$(uname -m).tar.gz" \
  | tar -xz
```

# 🤸 usage

This tool communicates with AWS Lambda and Resource tagging API's using the standard AWS credential chain
to authenticate requests. You may wish to export an `AWS_PROFILE` env variable to query your lambdas from different accounts.

The main usecase for this cli delving into your account to discover Lambdas of interest.

```sh
USAGE:
    lambstock <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    help    Prints this message or the help of the given subcommand(s)
    list    List lambdas
    tags    List lambdas tags
```

## tags

The approach this cli takes to to leverage a built-in feature of AWS for adding tags to Lambdas. A common case for this is
to tag a Lambda with a team name, or product, or some other arbitrary annotation. To see a list of tags run

```sh
$ lambstock tags
# ... list of tags associated with Lambda resources under your account
```

## list

You can use the `list` subcommand to discover Lambdas either as a raw list of filtered by tag

```sh
# all the lambdas
$ lambstock list
```

```sh
# all of my-awesome-teams lambdas
$ lambstock list --tag team=my-awesome-team
```

### sorting

You can also sort results based on `name`, `codesize` or `runtime`

```sh
# all of my-awesome-teams lambdas
$ lambstock list --tag team=my-awesome-team --sort codesize
```

# 👩‍🏭 development

This is a [rustlang](https://www.rust-lang.org/en-US/) application.
Go grab yourself a copy with [rustup](https://rustup.rs/).