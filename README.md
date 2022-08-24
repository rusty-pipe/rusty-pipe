# Rusty pipe

Rusty pipe - quick and rusty tool to port forward or reverse port forward between localhost,
containers and k8 pods.

## USAGE:
    rs [OPTIONS] [SUBCOMMAND]

## OPTIONS:
    -h, --help       Print help information
    -v               Output info log
    -V, --version    Print version information

## SUBCOMMANDS:
    completion    Output shell completion code
    help          Print this message or the help of the given subcommand(s)
    ls            List available endpoints
    pf            Port forward from ORIGIN to DESTINATION

## pf USAGE:
    rs pf <ORIGIN> <DESTINATION>

### ARGS:
    <ORIGIN>

    Available forward points are:
        Kubernetes: '<context>/<namespace>/<pod>:<PORT>'
        Docker: '<container>:<PORT>'
        Local: '[ADDR]:<PORT>'
        STDIO: '-'

    <DESTINATION>

    Available forward points are:
        Kubernetes: '<context>/<namespace>/<pod>:<PORT>'
        Docker: '<container>:<PORT>'
        Local: '[ADDR]:<PORT>'
        STDIO: '-'