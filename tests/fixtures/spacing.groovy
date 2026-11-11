package demo

import spock.lang.Specification

class Spacing<T extends Number>extends Specification{
    private static final Map<String,Integer> LIMITS=[low:1, high : 10]
    String  name      // aligned
    int     count     // comments

    Spacing( String name , int count ){
        this.name=name
        this.count = count
    }

    def total( List<T> xs ){
        def sum=0
        for(T x in xs){ sum+=x.intValue() }
        for (int i=0;i<xs.size();i++) { sum -= 0 }
        while(sum>100) sum--
        return sum>0?sum:-sum
    }

    def misc(a, b) {
        def m = [ name:'x', 'quoted key' :2 ]
        def l = [ 1,2,3 ]
        def r = 1..10
        def c = (String)a
        def d = b  as  String
        def e = a?:b
        def f = xs*.name
        def g = a?.b
        def h = String :: valueOf
        def k = { x , y->x+y }
        def p = - a
        def q = ! b
        if(a){ println a }else if(b){ println b }else{ println 'none' }
        try{ risky() }catch(IOException ex){ log.warn 'x' , ex }finally{ done() }
        switch(a){
            case 1 :
                break
            default :
                break
        }
        a -b
        foo (x).bar()
        y [1]
        println "keep ${a+b}   as is"
        def s = '''a  ,  b
   c  =  d'''  +'e'
        task (name:'x', type : Copy){ from 'a' }
        render(view:'show', model:[item:a])
    }

    def 'data table'() {
        expect:
        Math.max(a,b) == c

        where:
        a  | b  || c
        1  | 3  || 3
        7  | 4  || 7
    }
}
